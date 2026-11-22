//! ロックファイルから直接依存と、解決済みのバージョンを読む。

mod npm;
mod pnpm;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};

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

/// マニフェストに直接書かれていて、ロックファイルで解決済みのバージョンが分かる依存。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectDep {
    /// マニフェスト上の名前(エイリアスならエイリアス名)
    pub alias: String,
    /// レジストリ上の実パッケージ名
    pub name: String,
    pub version: String,
    pub kind: DepKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    PackageLock,
    PnpmLock,
}

impl Format {
    const ALL: [(Self, &str); 2] = [
        (Self::PnpmLock, "pnpm-lock.yaml"),
        (Self::PackageLock, "package-lock.json"),
    ];

    pub fn file_name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(f, _)| *f == self)
            .map(|(_, n)| *n)
            .unwrap_or_default()
    }
}

pub struct Lockfile {
    pub path: PathBuf,
    pub format: Format,
    pub deps: Vec<DirectDep>,
    pub warnings: Vec<String>,
}

impl Lockfile {
    /// ロックファイルのあるディレクトリ(`.npmrc` などの設定を探す場所)。
    pub fn project_dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }
}

/// ファイルならその名前から形式を決め、ディレクトリなら対応するロックファイルを探す。
pub fn find(path: &Path) -> Result<(PathBuf, Format)> {
    if path.is_dir() {
        let found: Vec<_> = Format::ALL
            .iter()
            .map(|(f, name)| (path.join(name), *f))
            .filter(|(p, _)| p.is_file())
            .collect();
        return match found.as_slice() {
            [one] => Ok(one.clone()),
            [] => bail!(
                "{} に対応するロックファイルがありません({})",
                path.display(),
                Format::ALL.map(|(_, n)| n).join(", ")
            ),
            _ => bail!(
                "{} にロックファイルが複数あります。どれを使うかファイルで指定してください",
                path.display()
            ),
        };
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    Format::ALL
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(f, _)| (path.to_path_buf(), *f))
        .with_context(|| format!("{} は対応していないファイルです", path.display()))
}

pub fn read(path: &Path) -> Result<Lockfile> {
    let (path, format) = find(path)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("{} を読めませんでした", path.display()))?;
    let (deps, warnings) = match format {
        Format::PackageLock => npm::parse(&text),
        Format::PnpmLock => pnpm::parse(&text),
    }
    .with_context(|| format!("{} を解析できませんでした", path.display()))?;
    Ok(Lockfile {
        path,
        format,
        deps,
        warnings,
    })
}
