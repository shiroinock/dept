use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DepKind {
    Prod,
    Optional,
    Dev,
}

impl DepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prod => "prod",
            Self::Optional => "optional",
            Self::Dev => "dev",
        }
    }
}

/// package.json に直接書かれていて、ロックファイルで解決済みのバージョンが分かる依存。
#[derive(Debug, Clone)]
pub struct DirectDep {
    /// package.json 上の名前(エイリアスならエイリアス名)
    pub alias: String,
    /// レジストリ上の実パッケージ名
    pub name: String,
    pub version: String,
    pub kind: DepKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lock {
    lockfile_version: Option<u32>,
    #[serde(default)]
    packages: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: Option<String>,
    version: Option<String>,
    #[serde(default)]
    link: bool,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    dev_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    optional_dependencies: BTreeMap<String, String>,
}

/// 警告(スキップした依存など)と一緒に直接依存を返す。
pub fn read_npm(path: &Path) -> Result<(Vec<DirectDep>, Vec<String>)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{} を読めませんでした", path.display()))?;
    parse_npm(&text).with_context(|| format!("{} を解析できませんでした", path.display()))
}

pub fn parse_npm(text: &str) -> Result<(Vec<DirectDep>, Vec<String>)> {
    let lock: Lock = serde_json::from_str(text)?;
    match lock.lockfile_version {
        Some(2 | 3) => {}
        Some(v) => bail!("lockfileVersion {v} には対応していません(2 と 3 に対応)"),
        None => bail!("lockfileVersion がありません"),
    }
    let Some(root) = lock.packages.get("") else {
        bail!("packages[\"\"](ルートパッケージ)がありません");
    };

    // 同じ名前が複数の欄にあるときは prod > optional > dev を優先する
    let mut wanted: BTreeMap<&str, DepKind> = BTreeMap::new();
    for (map, kind) in [
        (&root.dev_dependencies, DepKind::Dev),
        (&root.optional_dependencies, DepKind::Optional),
        (&root.dependencies, DepKind::Prod),
    ] {
        for name in map.keys() {
            wanted.insert(name, kind);
        }
    }

    let mut deps = Vec::new();
    let mut warnings = Vec::new();
    for (alias, kind) in wanted {
        let Some(entry) = lock.packages.get(&format!("node_modules/{alias}")) else {
            warnings.push(format!(
                "{alias}: ロックファイルにインストール済みの記録がありません"
            ));
            continue;
        };
        if entry.link {
            // ワークスペースやローカルパスの依存はレジストリと比べられない
            continue;
        }
        let Some(version) = entry.version.clone() else {
            warnings.push(format!("{alias}: バージョンがありません"));
            continue;
        };
        deps.push(DirectDep {
            alias: alias.to_string(),
            name: entry.name.clone().unwrap_or_else(|| alias.to_string()),
            version,
            kind,
        });
    }
    Ok((deps, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"{
      "name": "app",
      "lockfileVersion": 3,
      "packages": {
        "": {
          "name": "app",
          "dependencies": { "react": "^18.0.0", "my-lodash": "npm:lodash@^4", "local": "file:../local" },
          "devDependencies": { "typescript": "^5.0.0", "react": "^18.0.0" },
          "optionalDependencies": { "fsevents": "^2.3.0" }
        },
        "node_modules/react": { "version": "18.2.0" },
        "node_modules/my-lodash": { "name": "lodash", "version": "4.17.20" },
        "node_modules/local": { "resolved": "../local", "link": true },
        "node_modules/typescript": { "version": "5.1.6", "dev": true },
        "node_modules/loose-envify": { "version": "1.4.0" }
      }
    }"#;

    #[test]
    fn reads_direct_deps_only() {
        let (deps, warnings) = parse_npm(LOCK).unwrap();
        let got: Vec<_> = deps
            .iter()
            .map(|d| {
                (
                    d.alias.as_str(),
                    d.name.as_str(),
                    d.version.as_str(),
                    d.kind,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("my-lodash", "lodash", "4.17.20", DepKind::Prod),
                ("react", "react", "18.2.0", DepKind::Prod),
                ("typescript", "typescript", "5.1.6", DepKind::Dev),
            ]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}"); // fsevents は未インストール
    }

    #[test]
    fn rejects_v1() {
        let err = parse_npm(r#"{"lockfileVersion": 1, "dependencies": {}}"#).unwrap_err();
        assert!(err.to_string().contains("lockfileVersion 1"));
    }
}
