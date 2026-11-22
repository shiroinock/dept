//! deps.dev API(v3 GetPackage)のクライアント。応答は XDG キャッシュに保存する。

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const BASE: &str = "https://api.deps.dev/v3";
const MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, Deserialize)]
pub struct Package {
    #[serde(default)]
    pub versions: Vec<PackageVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageVersion {
    pub version_key: VersionKey,
    pub published_at: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
pub struct VersionKey {
    pub version: String,
}

pub struct Client {
    agent: ureq::Agent,
    cache_dir: Option<PathBuf>,
    ttl: Duration,
    offline: bool,
}

pub enum Fetched {
    Found(Package),
    NotFound,
}

impl Client {
    pub fn new(cache_dir: Option<PathBuf>, ttl: Duration, offline: bool) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("dept/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            agent,
            cache_dir,
            ttl,
            offline,
        }
    }

    pub fn npm_package(&self, name: &str) -> Result<Fetched> {
        let cache = self
            .cache_dir
            .as_ref()
            .map(|d| d.join("npm").join(cache_file_name(name)));

        let ttl = if self.offline { None } else { Some(self.ttl) };
        if let Some(path) = &cache
            && let Some(body) = read_fresh(path, ttl)
        {
            return parse(&body);
        }
        if self.offline {
            bail!("オフラインモードですがキャッシュがありません");
        }

        let url = format!("{BASE}/systems/npm/packages/{}", encode_path_segment(name));
        let body = self.get_with_retry(&url)?;
        if let (Some(path), Some(body)) = (&cache, &body) {
            write_cache(path, body);
        }
        match body {
            Some(b) => parse(&b),
            None => Ok(Fetched::NotFound),
        }
    }

    /// 200 なら本文、404 なら None。429 と 5xx は指数バックオフで再試行する。
    fn get_with_retry(&self, url: &str) -> Result<Option<String>> {
        let mut delay = Duration::from_millis(500);
        for attempt in 1..=MAX_ATTEMPTS {
            let result = self.agent.get(url).call();
            let retry_after = match result {
                Ok(mut resp) => {
                    let status = resp.status().as_u16();
                    match status {
                        200 => return Ok(Some(resp.body_mut().read_to_string()?)),
                        404 => return Ok(None),
                        429 | 500..=599 => resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.trim().parse::<u64>().ok())
                            .map(Duration::from_secs),
                        _ => bail!("deps.dev が HTTP {status} を返しました"),
                    }
                }
                // 接続エラーやタイムアウトも再試行する
                Err(_) if attempt < MAX_ATTEMPTS => None,
                Err(e) => return Err(anyhow!(e).context("deps.dev に接続できませんでした")),
            };
            if attempt == MAX_ATTEMPTS {
                break;
            }
            std::thread::sleep(retry_after.unwrap_or(delay).min(Duration::from_secs(60)));
            delay *= 2;
        }
        bail!("deps.dev への再試行が上限({MAX_ATTEMPTS} 回)に達しました")
    }
}

fn parse(body: &str) -> Result<Fetched> {
    let pkg = serde_json::from_str(body).context("deps.dev の応答を解析できませんでした")?;
    Ok(Fetched::Found(pkg))
}

/// `@scope/name` のような名前を 1 つのパスセグメントとして送れるように符号化する。
fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn cache_file_name(name: &str) -> String {
    format!("{}.json", encode_path_segment(name))
}

fn read_fresh(path: &PathBuf, ttl: Option<Duration>) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if let Some(ttl) = ttl {
        let age = SystemTime::now()
            .duration_since(meta.modified().ok()?)
            .unwrap_or_default();
        if age > ttl {
            return None;
        }
    }
    std::fs::read_to_string(path).ok()
}

fn write_cache(path: &PathBuf, body: &str) {
    // キャッシュの書き込み失敗は致命的ではないので無視する
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// `$XDG_CACHE_HOME/dept`、未設定なら `$HOME/.cache/dept`。
pub fn default_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("dept").join("depsdev"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_scoped_names() {
        assert_eq!(encode_path_segment("@types/node"), "%40types%2Fnode");
        assert_eq!(encode_path_segment("lodash.merge"), "lodash.merge");
    }

    #[test]
    fn parses_get_package_response() {
        let body = r#"{"packageKey":{"system":"NPM","name":"x"},"versions":[
            {"versionKey":{"system":"NPM","name":"x","version":"1.0.0"},"publishedAt":"2020-01-01T00:00:00Z","isDefault":false},
            {"versionKey":{"system":"NPM","name":"x","version":"1.1.0"},"isDefault":true}]}"#;
        let Fetched::Found(p) = parse(body).unwrap() else {
            panic!()
        };
        assert_eq!(p.versions.len(), 2);
        assert!(p.versions[1].is_default);
        assert!(p.versions[1].published_at.is_none());
    }
}
