//! バッジのリンク先にする Markdown のレポート。GitHub がそのまま表として表示する。
//!
//! 日時など実行ごとに変わるものは入れない(中身が同じなら push しないため)。

use std::fmt::Write;

use crate::{Report, Row};

pub fn markdown(report: &Report) -> String {
    let mut s = String::new();
    let w = report.weights;
    let l = report.lag;
    let _ = writeln!(s, "# dept: {}\n", report.total);
    let _ = writeln!(
        s,
        "依存ライブラリが最新版からどれだけ遅れているか。最新版までに出たリリースを major / minor / patch に分けて数え、重みを掛けて足したもの([スコアの定義](https://github.com/shiroinock/dept#スコアの定義))。\n"
    );
    let _ = writeln!(
        s,
        "- 合計 **{}** = major {} × {} + minor {} × {} + patch {} × {}",
        report.total, l.major, w.major, l.minor, w.minor, l.patch, w.patch
    );
    let _ = writeln!(
        s,
        "- {} / {} パッケージが遅れている、{:.1} libyears",
        report.outdated, report.packages, report.libyear
    );
    let _ = writeln!(s, "- ロックファイル:`{}`", report.lockfile);
    if report.errors > 0 {
        let _ = writeln!(
            s,
            "- 調べられなかった依存:{} 件(合計に入っていない)",
            report.errors
        );
    }
    s.push('\n');

    let outdated: Vec<&Row> = report.rows.iter().filter(|r| r.score > 0).collect();
    if outdated.is_empty() {
        s.push_str("遅れている依存はありません。\n");
    } else {
        s.push_str("| SCORE | PACKAGE | CURRENT | LATEST | MAJ | MIN | PAT | LIBYEAR | KIND |\n");
        s.push_str("| ---: | --- | --- | --- | ---: | ---: | ---: | ---: | --- |\n");
        for r in outdated {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                r.score,
                name(r),
                cell(&r.current),
                cell(r.latest.as_deref().unwrap_or("?")),
                r.lag.major,
                r.lag.minor,
                r.lag.patch,
                r.libyear
                    .map(|y| format!("{y:.1}"))
                    .unwrap_or_else(|| "-".into()),
                r.kind.as_str(),
            );
        }
    }

    let errors: Vec<&Row> = report.rows.iter().filter(|r| r.error.is_some()).collect();
    if !errors.is_empty() {
        s.push_str(
            "\n## 調べられなかった依存\n\n| PACKAGE | CURRENT | 理由 |\n| --- | --- | --- |\n",
        );
        for r in errors {
            let _ = writeln!(
                s,
                "| {} | {} | {} |",
                name(r),
                cell(&r.current),
                cell(r.error.as_deref().unwrap_or_default())
            );
        }
    }

    let latest: Vec<&Row> = report
        .rows
        .iter()
        .filter(|r| r.score == 0 && r.error.is_none())
        .collect();
    if !latest.is_empty() {
        let _ = writeln!(
            s,
            "\n<details>\n<summary>最新の依存({} 件)</summary>\n\n| PACKAGE | VERSION | KIND |\n| --- | --- | --- |",
            latest.len()
        );
        for r in latest {
            let _ = writeln!(
                s,
                "| {} | {} | {} |",
                name(r),
                cell(&r.current),
                r.kind.as_str()
            );
        }
        s.push_str("\n</details>\n");
    }

    s.push_str("\n---\n\n[dept](https://github.com/shiroinock/dept) で作成\n");
    s
}

fn name(r: &Row) -> String {
    if r.name == r.package {
        format!("`{}`", cell(&r.name))
    } else {
        format!("`{}`({})", cell(&r.name), cell(&r.package))
    }
}

/// 表のセルを壊す `|` と改行を避ける
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::DepKind;
    use crate::score::{Lag, Weights};

    fn row(name: &str, score: u64, error: Option<&str>) -> Row {
        Row {
            name: name.into(),
            package: name.into(),
            kind: DepKind::Prod,
            current: "1.0.0".into(),
            latest: Some("2.0.0".into()),
            lag: Lag {
                major: score / 30,
                minor: 0,
                patch: 0,
            },
            score,
            libyear: Some(1.25),
            error: error.map(Into::into),
        }
    }

    #[test]
    fn renders_sections() {
        let report = Report {
            lockfile: "pnpm-lock.yaml".into(),
            format: "pnpm-lock.yaml",
            weights: Weights::default(),
            total: 30,
            lag: Lag {
                major: 1,
                minor: 0,
                patch: 0,
            },
            packages: 3,
            outdated: 1,
            errors: 1,
            libyear: 1.25,
            rows: vec![
                row("react", 30, None),
                row("left-pad", 0, None),
                row("gone", 0, Some("見つから|ない")),
            ],
        };
        let md = markdown(&report);
        assert!(md.starts_with("# dept: 30\n"), "{md}");
        assert!(
            md.contains("| 30 | `react` | 1.0.0 | 2.0.0 | 1 | 0 | 0 | 1.2 | prod |"),
            "{md}"
        );
        assert!(md.contains("見つから\\|ない"), "{md}");
        assert!(md.contains("最新の依存(1 件)"), "{md}");
        assert!(md.contains("| `left-pad` | 1.0.0 | prod |"), "{md}");
        assert!(!md.contains("| 0 | `gone`"), "{md}");
    }
}
