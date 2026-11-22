//! npm レジストリ(npm と pnpm が使う)から、パッケージのリリース一覧を取る。
//!
//! レジストリは `.npmrc` の `registry=` と `@scope:registry=` に従う。
//! 優先順は 環境変数 `npm_config_registry` > プロジェクトの `.npmrc` > `~/.npmrc`。
//! 認証(`_authToken` など)にはまだ対応していない。

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::fetch::{Fetcher, encode_path_segment};
use crate::registry::{Release, Releases};

const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org/";

#[derive(Debug, Clone, PartialEq)]
pub struct NpmConfig {
    registry: String,
    scoped: HashMap<String, String>,
}

impl Default for NpmConfig {
    fn default() -> Self {
        Self {
            registry: DEFAULT_REGISTRY.into(),
            scoped: HashMap::new(),
        }
    }
}

impl NpmConfig {
    pub fn load(project_dir: &Path) -> Self {
        let mut cfg = Self::default();
        if let Some(home) = std::env::var_os("HOME") {
            cfg.apply_file(&Path::new(&home).join(".npmrc"));
        }
        cfg.apply_file(&project_dir.join(".npmrc"));
        if let Ok(r) = std::env::var("npm_config_registry")
            && !r.is_empty()
        {
            cfg.registry = with_trailing_slash(&r);
        }
        cfg
    }

    fn apply_file(&mut self, path: &Path) {
        if let Ok(text) = std::fs::read_to_string(path) {
            self.apply(&text);
        }
    }

    fn apply(&mut self, npmrc: &str) {
        for line in npmrc.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), expand_env(value.trim()));
            if key == "registry" {
                self.registry = with_trailing_slash(&value);
            } else if let Some(scope) = key.strip_suffix(":registry")
                && scope.starts_with('@')
            {
                self.scoped
                    .insert(scope.to_string(), with_trailing_slash(&value));
            }
        }
    }

    fn registry_for(&self, name: &str) -> &str {
        name.split_once('/')
            .and_then(|(scope, _)| self.scoped.get(scope))
            .unwrap_or(&self.registry)
    }
}

fn with_trailing_slash(s: &str) -> String {
    let s = s.trim_matches('"');
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

/// `${VAR}` を環境変数の値に置き換える(npm と同じ)。
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        let var = &rest[start + 2..start + len];
        out.push_str(&std::env::var(var).unwrap_or_default());
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

#[derive(Deserialize)]
struct Packument {
    #[serde(rename = "dist-tags", default)]
    dist_tags: HashMap<String, String>,
    #[serde(default)]
    versions: BTreeMap<String, IgnoredAny>,
    /// バージョンごとの公開日時。`created` / `modified` / `unpublished` も混ざる
    #[serde(default)]
    time: HashMap<String, serde_json::Value>,
}

pub fn releases(f: &Fetcher, cfg: &NpmConfig, name: &str) -> Result<Option<Releases>> {
    let registry = cfg.registry_for(name);
    let host = registry
        .split_once("://")
        .map_or(registry, |(_, rest)| rest)
        .trim_end_matches('/');
    f.cached(&["npm", host, name], |f| {
        // スコープ付きの名前は `@scope%2Fname` の形で送る
        let path = match name.strip_prefix('@') {
            Some(rest) => format!("@{}", encode_path_segment(rest)),
            None => encode_path_segment(name),
        };
        let Some(body) = f.get(
            &format!("{registry}{path}"),
            &[("accept", "application/json")],
        )?
        else {
            return Ok(None);
        };
        let doc: Packument = serde_json::from_str(&body)
            .with_context(|| format!("{name} のメタデータを解析できませんでした"))?;
        Ok(Some(to_releases(doc)))
    })
}

fn to_releases(doc: Packument) -> Releases {
    let versions = doc
        .versions
        .into_keys()
        .map(|version| Release {
            published: doc
                .time
                .get(&version)
                .and_then(|t| t.as_str())
                .map(String::from),
            version,
        })
        .collect();
    Releases {
        default: doc.dist_tags.get("latest").cloned(),
        versions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_registries_from_npmrc() {
        let mut cfg = NpmConfig::default();
        cfg.apply(
            "# comment\nregistry=https://npm.example.com\n@corp:registry = https://corp.example.com/npm/\n//corp.example.com/npm/:_authToken=${TOKEN}\n",
        );
        assert_eq!(cfg.registry_for("react"), "https://npm.example.com/");
        assert_eq!(
            cfg.registry_for("@corp/ui"),
            "https://corp.example.com/npm/"
        );
        assert_eq!(cfg.registry_for("@types/node"), "https://npm.example.com/");
    }

    #[test]
    fn expands_env_vars() {
        // SAFETY: テストの中だけで使う固有の名前
        unsafe { std::env::set_var("DEPT_TEST_HOST", "h.example.com") };
        assert_eq!(
            expand_env("https://${DEPT_TEST_HOST}/x"),
            "https://h.example.com/x"
        );
        assert_eq!(expand_env("plain"), "plain");
    }

    #[test]
    fn converts_packument() {
        let doc: Packument = serde_json::from_str(
            r#"{
              "dist-tags": {"latest": "1.1.0", "next": "2.0.0-rc.1"},
              "versions": {"1.0.0": {"name": "x"}, "1.1.0": {}, "2.0.0-rc.1": {}},
              "time": {"created": "2020-01-01T00:00:00.000Z", "1.0.0": "2020-01-01T00:00:00.000Z",
                       "1.1.0": "2021-01-01T00:00:00.000Z", "unpublished": {"time": "x"}}
            }"#,
        )
        .unwrap();
        let r = to_releases(doc);
        assert_eq!(r.default.as_deref(), Some("1.1.0"));
        assert_eq!(r.versions.len(), 3);
        let rc = r
            .versions
            .iter()
            .find(|v| v.version == "2.0.0-rc.1")
            .unwrap();
        assert!(rc.published.is_none());
    }
}
