//! レジストリへの HTTP 取得と、その結果の XDG キャッシュ。どのレジストリでも共通に使う。

use anyhow::{Result, anyhow, bail};
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAX_ATTEMPTS: u32 = 5;

pub struct Fetcher {
    agent: ureq::Agent,
    cache_dir: Option<PathBuf>,
    ttl: Duration,
    offline: bool,
}

impl Fetcher {
    pub fn new(cache_dir: Option<PathBuf>, ttl: Duration, offline: bool) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
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

    /// `key` のキャッシュが新しければそれを返し、なければ `fetch` で取ってキャッシュする。
    /// 見つからなかった(`None`)結果はキャッシュしない。
    pub fn cached<T, F>(&self, key: &[&str], fetch: F) -> Result<Option<T>>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce(&Self) -> Result<Option<T>>,
    {
        let path = self.cache_dir.as_ref().map(|dir| {
            let mut p = dir.clone();
            for part in key {
                p.push(encode_path_segment(part));
            }
            p.set_extension("json");
            p
        });

        let ttl = if self.offline { None } else { Some(self.ttl) };
        if let Some(path) = &path
            && let Some(body) = read_fresh(path, ttl)
            && let Ok(value) = serde_json::from_str(&body)
        {
            return Ok(Some(value));
        }
        if self.offline {
            bail!("オフラインモードですがキャッシュがありません");
        }

        let value = fetch(self)?;
        if let (Some(path), Some(value)) = (&path, &value)
            && let Ok(body) = serde_json::to_string(value)
        {
            write_cache(path, &body);
        }
        Ok(value)
    }

    /// 200 なら本文、404 なら None。429 と 5xx と接続エラーは指数バックオフで再試行する。
    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Option<String>> {
        let mut delay = Duration::from_millis(500);
        for attempt in 1..=MAX_ATTEMPTS {
            let mut req = self.agent.get(url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            let retry_after = match req.call() {
                Ok(mut resp) => match resp.status().as_u16() {
                    200 => {
                        let body = resp
                            .body_mut()
                            .with_config()
                            .limit(256 * 1024 * 1024)
                            .read_to_string()?;
                        return Ok(Some(body));
                    }
                    404 => return Ok(None),
                    429 | 500..=599 => resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map(Duration::from_secs),
                    status => bail!("{url} が HTTP {status} を返しました"),
                },
                Err(_) if attempt < MAX_ATTEMPTS => None,
                Err(e) => return Err(anyhow!(e).context(format!("{url} に接続できませんでした"))),
            };
            if attempt == MAX_ATTEMPTS {
                break;
            }
            std::thread::sleep(retry_after.unwrap_or(delay).min(Duration::from_secs(60)));
            delay *= 2;
        }
        bail!("{url} への再試行が上限({MAX_ATTEMPTS} 回)に達しました")
    }
}

/// `@scope/name` のような文字列を、URL やファイル名の 1 つのセグメントとして使えるように符号化する。
pub fn encode_path_segment(s: &str) -> String {
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

fn read_fresh(path: &Path, ttl: Option<Duration>) -> Option<String> {
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

fn write_cache(path: &Path, body: &str) {
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
    // v1 は deps.dev の生の応答を置いていた場所。形式が違うので分ける
    Some(base.join("dept").join("v2"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_scoped_names() {
        assert_eq!(encode_path_segment("@types/node"), "%40types%2Fnode");
        assert_eq!(encode_path_segment("lodash.merge"), "lodash.merge");
    }
}
