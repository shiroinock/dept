//! README に貼るバッジ(shields.io の flat と同じ見た目の SVG)を外部サービスなしで作る。

use anyhow::{Result, bail};
use std::str::FromStr;

/// 色を変える境目。`green` 以下は緑、`yellow` 以下は黄、それより上は赤。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub green: u64,
    pub yellow: u64,
}

impl FromStr for Thresholds {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        let [g, y] = parts.as_slice() else {
            bail!("GREEN,YELLOW の形で指定してください: {s}");
        };
        let (green, yellow): (u64, u64) = (g.parse()?, y.parse()?);
        if green > yellow {
            bail!("GREEN は YELLOW 以下にしてください: {s}");
        }
        Ok(Self { green, yellow })
    }
}

impl Thresholds {
    fn color(self, score: u64) -> &'static str {
        if score <= self.green {
            "#4c1"
        } else if score <= self.yellow {
            "#dfb317"
        } else {
            "#e05d44"
        }
    }
}

pub fn render(label: &str, score: u64, thresholds: Thresholds) -> String {
    let value = score.to_string();
    let color = thresholds.color(score);
    // 文字の幅 + 左右 5px ずつの余白
    let lw = (text_width(label) + 10.0).round();
    let vw = (text_width(&value) + 10.0).round();
    let w = lw + vw;
    let (lx, vx) = (lw / 2.0, lw + vw / 2.0);
    let label = escape(label);
    let title = format!("{label}: {value}");
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="20" role="img" aria-label="{title}"><title>{title}</title><linearGradient id="s" x2="0" y2="100%"><stop offset="0" stop-color="#bbb" stop-opacity=".1"/><stop offset="1" stop-opacity=".1"/></linearGradient><clipPath id="r"><rect width="{w}" height="20" rx="3" fill="#fff"/></clipPath><g clip-path="url(#r)"><rect width="{lw}" height="20" fill="#555"/><rect x="{lw}" width="{vw}" height="20" fill="{color}"/><rect width="{w}" height="20" fill="url(#s)"/></g><g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11"><text x="{lx}" y="15" fill="#010101" fill-opacity=".3">{label}</text><text x="{lx}" y="14">{label}</text><text x="{vx}" y="15" fill="#010101" fill-opacity=".3">{value}</text><text x="{vx}" y="14">{value}</text></g></svg>
"##
    )
}

/// Verdana 11px でのおおよその幅。フォントを読まずに済むよう、よく使う文字だけ表にしている。
fn text_width(s: &str) -> f64 {
    s.chars()
        .map(|c| match c {
            '0'..='9' => 7.0,
            'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '!' | '|' | '\'' => 3.1,
            'f' | 't' | 'r' | 'I' | ' ' => 4.3,
            'm' | 'w' => 9.9,
            'M' | 'W' => 11.0,
            'A'..='Z' => 7.5,
            'a'..='z' => 6.6,
            '-' | '_' => 4.8,
            c if c.is_ascii() => 7.0,
            // 日本語などの全角
            _ => 11.0,
        })
        .sum()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Thresholds = Thresholds {
        green: 100,
        yellow: 500,
    };

    #[test]
    fn parses_thresholds() {
        assert_eq!("100, 500".parse::<Thresholds>().unwrap(), T);
        assert!("500,100".parse::<Thresholds>().is_err());
        assert!("100".parse::<Thresholds>().is_err());
    }

    #[test]
    fn picks_color() {
        assert_eq!(T.color(0), "#4c1");
        assert_eq!(T.color(100), "#4c1");
        assert_eq!(T.color(101), "#dfb317");
        assert_eq!(T.color(501), "#e05d44");
    }

    #[test]
    fn renders_svg() {
        let svg = render("a&b", 1485, T);
        assert!(svg.starts_with("<svg "), "{svg}");
        assert!(svg.contains(r#"aria-label="a&amp;b: 1485""#), "{svg}");
        assert!(svg.contains("#e05d44"), "{svg}");
        assert!(!svg.contains("a&b"), "{svg}");
    }
}
