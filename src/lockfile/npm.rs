//! npm の package-lock.json(lockfileVersion 2 / 3)。

use anyhow::{Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

use super::{DepKind, DirectDep};

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
pub fn parse(text: &str) -> Result<(Vec<DirectDep>, Vec<String>)> {
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
        let (deps, warnings) = parse(LOCK).unwrap();
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
        let err = parse(r#"{"lockfileVersion": 1, "dependencies": {}}"#).unwrap_err();
        assert!(err.to_string().contains("lockfileVersion 1"));
    }
}
