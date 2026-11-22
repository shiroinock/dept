mod badge;
mod fetch;
mod lockfile;
mod npm;
mod registry;
mod report;
mod score;

use anyhow::{Context, Result};
use clap::Parser;
use semver::Version;
use serde::Serialize;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use badge::Thresholds;
use fetch::Fetcher;
use lockfile::{DepKind, DirectDep};
use npm::NpmConfig;
use registry::Releases;
use score::{Lag, Weights};

/// 依存ライブラリが最新版からどれだけ遅れているかをスコアにする。
///
/// 現在のバージョンから最新の安定版までに出たリリースを major / minor / patch に分けて数え、
/// 重み(既定 30 / 3 / 1)を掛けて足し合わせる。0.x の minor 上げは major として数える。
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// ロックファイル(pnpm-lock.yaml / package-lock.json)か、それを含むディレクトリ
    #[arg(default_value = ".")]
    path: PathBuf,

    /// 重み(MAJOR,MINOR,PATCH)
    #[arg(long, default_value = "30,3,1")]
    weights: Weights,

    /// devDependencies を除く
    #[arg(long)]
    prod: bool,

    /// 結果を JSON で出す
    #[arg(long)]
    json: bool,

    /// 最新の依存も表に出す
    #[arg(long)]
    all: bool,

    /// 表に出す件数の上限
    #[arg(long)]
    top: Option<usize>,

    /// 合計スコアがこの値を超えたら終了コード 2 で終わる
    #[arg(long, value_name = "N")]
    fail_over: Option<u64>,

    /// README に貼るバッジ(SVG)をこのファイルに書く
    #[arg(long, value_name = "FILE")]
    badge: Option<PathBuf>,

    /// どの依存がどれだけ遅れているかの Markdown のレポートをこのファイルに書く
    #[arg(long, value_name = "FILE")]
    report: Option<PathBuf>,

    /// バッジの左側の文字
    #[arg(long, default_value = "dept", requires = "badge")]
    badge_label: String,

    /// バッジの色の境目(GREEN,YELLOW)。GREEN 以下は緑、YELLOW 以下は黄、それより上は赤
    #[arg(long, default_value = "100,500", requires = "badge")]
    badge_colors: Thresholds,

    /// レジストリへ同時に送るリクエスト数
    #[arg(long, default_value_t = 8)]
    concurrency: usize,

    /// キャッシュの有効期間(時間)
    #[arg(long, default_value_t = 12)]
    cache_ttl: u64,

    /// キャッシュだけを使い、ネットワークに出ない(期限切れのキャッシュも使う)
    #[arg(long)]
    offline: bool,

    /// キャッシュを使わない
    #[arg(long, conflicts_with = "offline")]
    no_cache: bool,
}

#[derive(Serialize)]
struct Row {
    name: String,
    package: String,
    kind: DepKind,
    current: String,
    latest: Option<String>,
    lag: Lag,
    score: u64,
    libyear: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct Report {
    lockfile: String,
    format: &'static str,
    weights: Weights,
    total: u64,
    lag: Lag,
    packages: usize,
    outdated: usize,
    /// レジストリのエラーなどで調べられなかった依存の数。合計スコアには入っていない
    errors: usize,
    libyear: f64,
    rows: Vec<Row>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("dept: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let mut lock = lockfile::read(&cli.path)?;
    for w in &lock.warnings {
        eprintln!("warning: {w}");
    }
    if cli.prod {
        lock.deps.retain(|d| d.kind != DepKind::Dev);
    }

    let cache_dir = if cli.no_cache {
        None
    } else {
        fetch::default_cache_dir()
    };
    let sources = Sources {
        fetcher: Fetcher::new(
            cache_dir,
            Duration::from_secs(cli.cache_ttl * 3600),
            cli.offline,
        ),
        npm: NpmConfig::load(lock.project_dir()),
    };
    let mut rows = evaluate_all(&sources, &lock.deps, cli.weights, cli.concurrency.max(1));
    rows.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.current.cmp(&b.current))
    });

    let lag = rows.iter().fold(Lag::default(), |acc, r| Lag {
        major: acc.major + r.lag.major,
        minor: acc.minor + r.lag.minor,
        patch: acc.patch + r.lag.patch,
    });
    let report = Report {
        lockfile: lock.path.display().to_string(),
        format: lock.format.file_name(),
        weights: cli.weights,
        total: rows.iter().map(|r| r.score).sum(),
        lag,
        packages: rows.len(),
        outdated: rows.iter().filter(|r| r.score > 0).count(),
        errors: rows.iter().filter(|r| r.error.is_some()).count(),
        libyear: rows.iter().filter_map(|r| r.libyear).sum(),
        rows,
    };

    for r in report.rows.iter().filter(|r| r.error.is_some()) {
        eprintln!(
            "warning: {}: {}",
            r.name,
            r.error.as_deref().unwrap_or_default()
        );
    }

    if let Some(path) = &cli.badge {
        let svg = badge::render(&cli.badge_label, report.total, cli.badge_colors);
        std::fs::write(path, svg).with_context(|| format!("{} に書けません", path.display()))?;
    }

    if let Some(path) = &cli.report {
        std::fs::write(path, report::markdown(&report))
            .with_context(|| format!("{} に書けません", path.display()))?;
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_table(&report, cli.all, cli.top);
    }

    Ok(match cli.fail_over {
        Some(limit) if report.total > limit => {
            eprintln!(
                "dept: 合計スコア {} が上限 {limit} を超えています",
                report.total
            );
            ExitCode::from(2)
        }
        _ => ExitCode::SUCCESS,
    })
}

/// リリース一覧の取り先。いまは npm レジストリだけ。
struct Sources {
    fetcher: Fetcher,
    npm: NpmConfig,
}

impl Sources {
    fn releases(&self, dep: &DirectDep) -> Result<Option<Releases>> {
        npm::releases(&self.fetcher, &self.npm, &dep.name)
    }
}

fn evaluate_all(sources: &Sources, deps: &[DirectDep], w: Weights, workers: usize) -> Vec<Row> {
    let next = AtomicUsize::new(0);
    let rows = Mutex::new(Vec::with_capacity(deps.len()));
    std::thread::scope(|s| {
        for _ in 0..workers.min(deps.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(dep) = deps.get(i) else { break };
                    let row = evaluate(sources, dep, w);
                    rows.lock().unwrap().push(row);
                }
            });
        }
    });
    rows.into_inner().unwrap()
}

fn evaluate(sources: &Sources, dep: &DirectDep, w: Weights) -> Row {
    let mut row = Row {
        name: dep.alias.clone(),
        package: dep.name.clone(),
        kind: dep.kind,
        current: dep.version.clone(),
        latest: None,
        lag: Lag::default(),
        score: 0,
        libyear: None,
        error: None,
    };
    let current = match Version::parse(&dep.version) {
        Ok(v) => v,
        Err(e) => {
            row.error = Some(format!("バージョン {} を解釈できません: {e}", dep.version));
            return row;
        }
    };
    let pkg = match sources.releases(dep) {
        Ok(Some(p)) => p,
        Ok(None) => {
            row.error = Some("レジストリにパッケージが見つかりません".into());
            return row;
        }
        Err(e) => {
            row.error = Some(format!("{e:#}"));
            return row;
        }
    };

    let parsed: Vec<(Version, Option<&str>)> = pkg
        .versions
        .iter()
        .filter_map(|r| {
            Version::parse(&r.version)
                .ok()
                .map(|v| (v, r.published.as_deref()))
        })
        .collect();
    let releases: Vec<Version> = parsed.iter().map(|(v, _)| v.clone()).collect();
    let default = pkg.default.as_deref().and_then(|d| Version::parse(d).ok());
    let Some(target) = score::pick_target(default.as_ref(), &releases) else {
        row.error = Some("安定版のリリースがありません".into());
        return row;
    };

    row.lag = score::lag(&current, &target, &releases);
    row.score = row.lag.score(w);
    row.libyear = libyear(&parsed, &current, &target);
    row.latest = Some(target.to_string());
    row
}

/// 現在のバージョンと最新版の公開日の差(年)。どちらかの公開日が不明なら None。
fn libyear(parsed: &[(Version, Option<&str>)], current: &Version, target: &Version) -> Option<f64> {
    let published = |v: &Version| {
        parsed
            .iter()
            .find(|(pv, _)| pv == v)
            .and_then(|(_, p)| *p)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    };
    let days = (published(target)? - published(current)?).num_seconds() as f64 / 86_400.0;
    Some((days / 365.25).max(0.0))
}

fn print_table(report: &Report, all: bool, top: Option<usize>) {
    let shown: Vec<&Row> = report
        .rows
        .iter()
        .filter(|r| all || r.score > 0)
        .take(top.unwrap_or(usize::MAX))
        .collect();

    if !shown.is_empty() {
        let headers = [
            "SCORE", "PACKAGE", "CURRENT", "LATEST", "MAJ", "MIN", "PAT", "LIBYEAR", "KIND",
        ];
        let cells: Vec<[String; 9]> = shown
            .iter()
            .map(|r| {
                let name = if r.name == r.package {
                    r.name.clone()
                } else {
                    format!("{} ({})", r.name, r.package)
                };
                [
                    r.score.to_string(),
                    name,
                    r.current.clone(),
                    r.latest.clone().unwrap_or_else(|| "?".into()),
                    r.lag.major.to_string(),
                    r.lag.minor.to_string(),
                    r.lag.patch.to_string(),
                    r.libyear
                        .map(|y| format!("{y:.1}"))
                        .unwrap_or_else(|| "-".into()),
                    r.kind.as_str().to_string(),
                ]
            })
            .collect();
        let mut widths = headers.map(str::len);
        for row in &cells {
            for (w, c) in widths.iter_mut().zip(row) {
                *w = (*w).max(c.chars().count());
            }
        }
        // 数値の列は右寄せ
        let right = [true, false, false, false, true, true, true, true, false];
        let line = |row: &[String]| {
            row.iter()
                .zip(widths)
                .zip(right)
                .map(|((c, w), r)| {
                    if r {
                        format!("{c:>w$}")
                    } else {
                        format!("{c:<w$}")
                    }
                })
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_string()
        };
        println!("{}", line(&headers.map(String::from)));
        for row in &cells {
            println!("{}", line(row));
        }
        println!();
    }

    let w = report.weights;
    println!(
        "total: {}  (major {} × {} + minor {} × {} + patch {} × {})",
        report.total,
        report.lag.major,
        w.major,
        report.lag.minor,
        w.minor,
        report.lag.patch,
        w.patch
    );
    println!(
        "{} / {} packages outdated, {:.1} libyears",
        report.outdated, report.packages, report.libyear
    );
}
