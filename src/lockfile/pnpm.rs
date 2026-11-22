//! pnpm の pnpm-lock.yaml(lockfileVersion 6 / 9)。
//!
//! ワークスペースでは `importers` の各パッケージの直接依存を集める。同じパッケージの同じ
//! バージョンが複数の importer にあれば 1 つにまとめ、種類は prod > optional > dev を優先する。

use anyhow::{Result, bail};
use std::collections::BTreeMap;
use yaml_rust2::{Yaml, YamlLoader};

use super::{DepKind, DirectDep};

const SECTIONS: [(&str, DepKind); 3] = [
    ("dependencies", DepKind::Prod),
    ("optionalDependencies", DepKind::Optional),
    ("devDependencies", DepKind::Dev),
];

pub fn parse(text: &str) -> Result<(Vec<DirectDep>, Vec<String>)> {
    let docs = YamlLoader::load_from_str(text)?;
    let Some(doc) = docs.first() else {
        bail!("中身が空です");
    };

    let version = match &doc["lockfileVersion"] {
        Yaml::String(s) => s.parse::<f64>().ok(),
        Yaml::Real(s) => s.parse::<f64>().ok(),
        Yaml::Integer(i) => Some(*i as f64),
        _ => None,
    };
    match version {
        Some(v) if v >= 6.0 => {}
        Some(v) => bail!("lockfileVersion {v} には対応していません(6 以上に対応)"),
        None => bail!("lockfileVersion がありません"),
    }

    // ワークスペースでなければ、v6 は依存をトップレベルに書くことがある
    let importers: Vec<&Yaml> = match doc["importers"].as_hash() {
        Some(h) => h.values().collect(),
        None => vec![doc],
    };

    let mut found: BTreeMap<(String, String, String), DepKind> = BTreeMap::new();
    let mut warnings = Vec::new();
    for importer in importers {
        for (section, kind) in SECTIONS {
            let Some(deps) = importer[section].as_hash() else {
                continue;
            };
            for (alias, entry) in deps {
                let Some(alias) = alias.as_str() else {
                    continue;
                };
                let raw = entry["version"].as_str().or_else(|| entry.as_str());
                let Some(raw) = raw else {
                    warnings.push(format!("{alias}: バージョンがありません"));
                    continue;
                };
                match resolve(alias, raw) {
                    Resolved::Registry { name, version } => {
                        let k = found
                            .entry((alias.to_string(), name, version))
                            .or_insert(kind);
                        *k = (*k).min(kind);
                    }
                    Resolved::Workspace => {}
                    Resolved::Other => {
                        warnings.push(format!(
                            "{alias}: レジストリ以外から入れている依存は対象外です({raw})"
                        ));
                    }
                }
            }
        }
    }

    let deps = found
        .into_iter()
        .map(|((alias, name, version), kind)| DirectDep {
            alias,
            name,
            version,
            kind,
        })
        .collect();
    Ok((deps, warnings))
}

#[derive(Debug, PartialEq)]
enum Resolved {
    Registry {
        name: String,
        version: String,
    },
    /// ワークスペース内のパッケージ(`link:`)。レジストリと比べられないので黙って外す
    Workspace,
    /// `file:`、tarball の URL、git など
    Other,
}

/// importer の `version:` を読む。ピアの解決結果 `(react@18.3.1)` は外し、
/// エイリアス(`npm:other@^1`)は `other@1.2.3` の形で書かれるので実名を取り出す。
fn resolve(alias: &str, raw: &str) -> Resolved {
    let base = raw.split('(').next().unwrap_or(raw);
    if base.starts_with("link:") {
        return Resolved::Workspace;
    }
    // file:、https://…、git+ssh:… など
    if base.contains(':') {
        return Resolved::Other;
    }
    // 先頭の `@` はスコープなので、2 文字目以降の `@` があればエイリアス
    match base.rfind('@').filter(|&i| i > 0) {
        Some(i) => Resolved::Registry {
            name: base[..i].to_string(),
            version: base[i + 1..].to_string(),
        },
        None => Resolved::Registry {
            name: alias.to_string(),
            version: base.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V9: &str = r#"
lockfileVersion: '9.0'

settings:
  autoInstallPeers: true

importers:

  .:
    dependencies:
      hono:
        specifier: ^4.12.9
        version: 4.12.9
      honox:
        specifier: ^0.1.55
        version: 0.1.55(hono@4.12.9)(wrangler@4.54.0(@cloudflare/workers-types@4.20260317.1))
      my-lodash:
        specifier: npm:lodash@^4
        version: lodash@4.17.21
      '@corp/ui':
        specifier: npm:@corp/design@^2
        version: '@corp/design@2.3.0'
      ocr:
        specifier: file:modules/ocr
        version: file:modules/ocr
    devDependencies:
      '@types/node':
        specifier: ^25.5.0
        version: 25.5.0
      hono:
        specifier: ^4.12.9
        version: 4.12.9

  packages/core:
    dependencies:
      '@app/common':
        specifier: workspace:*
        version: link:../common
      '@types/node':
        specifier: ^24.0.0
        version: 24.1.0
    devDependencies:
      '@types/node':
        specifier: ^25.5.0
        version: 25.5.0

packages:

  hono@4.12.9:
    resolution: {integrity: sha512-x}
"#;

    #[test]
    fn reads_importers_of_v9() {
        let (deps, warnings) = parse(V9).unwrap();
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
                ("@corp/ui", "@corp/design", "2.3.0", DepKind::Prod),
                ("@types/node", "@types/node", "24.1.0", DepKind::Prod),
                ("@types/node", "@types/node", "25.5.0", DepKind::Dev),
                ("hono", "hono", "4.12.9", DepKind::Prod),
                ("honox", "honox", "0.1.55", DepKind::Prod),
                ("my-lodash", "lodash", "4.17.21", DepKind::Prod),
            ]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}"); // file: の ocr
    }

    #[test]
    fn reads_top_level_deps_of_v6() {
        let text = "lockfileVersion: '6.0'\n\ndependencies:\n  react:\n    specifier: ^18.2.0\n    version: 18.3.1\n\ndevDependencies:\n  typescript:\n    specifier: ^5.0.0\n    version: 5.4.5\n";
        let (deps, _) = parse(text).unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "react");
        assert_eq!(deps[1].kind, DepKind::Dev);
    }

    #[test]
    fn rejects_v5() {
        let err = parse("lockfileVersion: 5.4\nspecifiers: {}\n").unwrap_err();
        assert!(err.to_string().contains("5.4"), "{err}");
    }

    #[test]
    fn resolves_versions() {
        assert_eq!(resolve("x", "link:../x"), Resolved::Workspace);
        assert_eq!(resolve("x", "https://example.com/x.tgz"), Resolved::Other);
        assert_eq!(
            resolve("@a/b", "1.0.0(react@18.0.0)"),
            Resolved::Registry {
                name: "@a/b".into(),
                version: "1.0.0".into()
            }
        );
    }
}
