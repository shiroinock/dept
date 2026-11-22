//! レジストリから取ったリリースの一覧。エコシステムによらない形にそろえる。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Releases {
    /// バージョン指定なしで入るもの(npm の `latest` タグ)。無ければ None。
    pub default: Option<String>,
    pub versions: Vec<Release>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    /// RFC 3339 の公開日時。レジストリが返さなければ None。
    pub published: Option<String>,
}
