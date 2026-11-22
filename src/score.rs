use semver::Version;
use serde::Serialize;

/// 各種リリース1回あたりの重み。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Weights {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            major: 30,
            minor: 3,
            patch: 1,
        }
    }
}

impl std::str::FromStr for Weights {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = s.split(',').map(str::trim).collect();
        let [major, minor, patch] = parts.as_slice() else {
            return Err("重みは MAJOR,MINOR,PATCH の形で指定してください(例: 30,3,1)".into());
        };
        let parse = |v: &str| v.parse::<u64>().map_err(|e| format!("{v}: {e}"));
        Ok(Self {
            major: parse(major)?,
            minor: parse(minor)?,
            patch: parse(patch)?,
        })
    }
}

/// 現在のバージョンから目標バージョンまでに出たリリースの内訳。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Lag {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Lag {
    pub fn score(&self, w: Weights) -> u64 {
        self.major * w.major + self.minor * w.minor + self.patch * w.patch
    }
}

/// 一つ前のリリースからの変化を分類する。
/// 0.x では semver の慣習どおり minor の変化を破壊的変更(major)とみなす。
fn classify(prev: &Version, next: &Version) -> Option<Kind> {
    if next.major != prev.major {
        Some(Kind::Major)
    } else if next.minor != prev.minor {
        Some(if next.major == 0 {
            Kind::Major
        } else {
            Kind::Minor
        })
    } else if next.patch != prev.patch || next.pre != prev.pre {
        Some(Kind::Patch)
    } else {
        None
    }
}

enum Kind {
    Major,
    Minor,
    Patch,
}

/// `current` より新しく `target` 以下の安定版リリースを、直前のリリースとの差で分類して数える。
pub fn lag(current: &Version, target: &Version, releases: &[Version]) -> Lag {
    let mut between: Vec<&Version> = releases
        .iter()
        .filter(|v| v.pre.is_empty() && *v > current && *v <= target)
        .collect();
    between.sort();
    between.dedup();

    let mut out = Lag::default();
    let mut prev = current;
    for v in between {
        match classify(prev, v) {
            Some(Kind::Major) => out.major += 1,
            Some(Kind::Minor) => out.minor += 1,
            Some(Kind::Patch) => out.patch += 1,
            None => {}
        }
        prev = v;
    }
    out
}

/// 目標バージョンを決める。レジストリのデフォルト(npm の latest タグ)が安定版ならそれを、
/// そうでなければ最大の安定版を使う。
pub fn pick_target(default: Option<&Version>, releases: &[Version]) -> Option<Version> {
    match default {
        Some(d) if d.pre.is_empty() => Some(d.clone()),
        _ => releases.iter().filter(|v| v.pre.is_empty()).max().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn vs(list: &[&str]) -> Vec<Version> {
        list.iter().map(|s| v(s)).collect()
    }

    #[test]
    fn counts_releases_by_kind() {
        let releases = vs(&[
            "1.2.3", "1.2.4", "1.3.0", "1.3.1", "2.0.0", "2.0.1", "2.1.0", "3.0.0", "3.1.0",
        ]);
        let l = lag(&v("1.2.3"), &v("3.1.0"), &releases);
        // 1.2.4(p) 1.3.0(m) 1.3.1(p) 2.0.0(M) 2.0.1(p) 2.1.0(m) 3.0.0(M) 3.1.0(m)
        assert_eq!(
            l,
            Lag {
                major: 2,
                minor: 3,
                patch: 3
            }
        );
        assert_eq!(l.score(Weights::default()), 2 * 30 + 3 * 3 + 3);
    }

    #[test]
    fn up_to_date_is_zero() {
        let releases = vs(&["1.0.0", "1.1.0"]);
        assert_eq!(lag(&v("1.1.0"), &v("1.1.0"), &releases), Lag::default());
        // 最新より新しいものを入れている場合も 0
        assert_eq!(lag(&v("2.0.0"), &v("1.1.0"), &releases), Lag::default());
    }

    #[test]
    fn ignores_prereleases_and_versions_beyond_target() {
        let releases = vs(&["1.0.0", "1.1.0-beta.1", "1.1.0", "2.0.0-rc.1", "2.0.0"]);
        let l = lag(&v("1.0.0"), &v("1.1.0"), &releases);
        assert_eq!(
            l,
            Lag {
                major: 0,
                minor: 1,
                patch: 0
            }
        );
    }

    #[test]
    fn zero_major_minor_bump_is_breaking() {
        let releases = vs(&["0.2.0", "0.2.1", "0.3.0", "0.4.0", "1.0.0"]);
        let l = lag(&v("0.2.0"), &v("1.0.0"), &releases);
        assert_eq!(
            l,
            Lag {
                major: 3,
                minor: 0,
                patch: 1
            }
        );
    }

    #[test]
    fn prerelease_current_to_its_release_is_patch() {
        let releases = vs(&["2.0.0-beta.1", "2.0.0"]);
        let l = lag(&v("2.0.0-beta.1"), &v("2.0.0"), &releases);
        assert_eq!(
            l,
            Lag {
                major: 0,
                minor: 0,
                patch: 1
            }
        );
    }

    #[test]
    fn target_prefers_stable_default() {
        let releases = vs(&["1.0.0", "2.0.0", "3.0.0-rc.1"]);
        assert_eq!(pick_target(Some(&v("1.0.0")), &releases), Some(v("1.0.0")));
        assert_eq!(
            pick_target(Some(&v("3.0.0-rc.1")), &releases),
            Some(v("2.0.0"))
        );
        assert_eq!(pick_target(None, &releases), Some(v("2.0.0")));
    }

    #[test]
    fn parses_weights() {
        let w: Weights = "10, 2, 0".parse().unwrap();
        assert_eq!((w.major, w.minor, w.patch), (10, 2, 0));
        assert!("1,2".parse::<Weights>().is_err());
    }
}
