# dept

依存ライブラリが最新版からどれだけ遅れているかを、重み付きのスコアで表す CLI。
名前は dep(依存)と debt(負債)を重ねたもの。

アップデートの影響の大きさと優先順位を判断する指標として使う。
重みは恣意的でよく、数字が大きい依存から上げる、合計の推移を追う、といった使い方を想定している。

## 元ネタ

[gotanda.sre#1](https://gotanda-sre.connpass.com/event/402441/)(2026-09-18)での
寺井 省吾さん(株式会社YOUTRUST、[@krpk1900_dev](https://x.com/krpk1900_dev))の発表
「インフラ、バックエンド、フロントエンドのすべてを最新にする方法」で紹介された、
ライブラリアップグレードスコアの考え方を元にしている。既定の重み(major 30 / minor 3 / patch 1)もこの発表による。

## スコアの定義

現在のバージョンから最新の安定版までに実際に公開されたリリースを、直前のリリースとの差で
major / minor / patch に分類して数え、重みを掛けて足す。

```
score = major × 30 + minor × 3 + patch × 1   # 既定の重み
```

- 最新版はレジストリの既定バージョン(npm の `latest` タグ)。それが prerelease なら最大の安定版を使う。
- prerelease(`-beta` など)は数えない。
- `0.x` の minor 上げは semver の慣習どおり破壊的変更とみなし、major として数える。
- 古い系列へのバックポート(`7.3.x` が `8.0.0` より後に出た場合など)も、semver の順序で間に入るものは数える。
- 参考として libyear(現在のバージョンと最新版の公開日の差、年単位)も表示する。

## 使い方

```bash
nix run github:shiroinock/dept -- path/to/project
```

```
SCORE  PACKAGE     CURRENT  LATEST  MAJ  MIN  PAT  LIBYEAR  KIND
   68  react       18.3.1   19.3.1    1    3   29      2.5  prod
   10  zustand     5.0.9    5.1.0     0    1    7      0.9  prod

total: 78  (major 1 × 30 + minor 4 × 3 + patch 36 × 1)
2 / 3 packages outdated, 3.4 libyears
```

| オプション | 説明 |
| --- | --- |
| `[PATH]` | `package-lock.json` か、それを含むディレクトリ(既定は `.`) |
| `--weights 30,3,1` | 重み(MAJOR,MINOR,PATCH) |
| `--prod` | devDependencies を除く |
| `--json` | JSON で出力する(CI で保存して推移を見る用) |
| `--all` / `--top N` | 最新の依存も表示する / 表示件数を絞る |
| `--fail-over N` | 合計スコアが N を超えたら終了コード 2 |
| `--concurrency 8` | deps.dev への同時リクエスト数 |
| `--cache-ttl 12` | キャッシュの有効期間(時間) |
| `--offline` / `--no-cache` | キャッシュだけを使う / キャッシュを使わない |

## 対応状況

- npm の `package-lock.json`(lockfileVersion 2 / 3)の直接依存。ワークスペースやローカルパスの依存は対象外。
- バージョン情報は [deps.dev API](https://docs.deps.dev/api/v3/) の `GetPackage` から取る。
  応答は `$XDG_CACHE_HOME/dept/`(未設定なら `~/.cache/dept/`)にキャッシュし、
  429 / 5xx は指数バックオフで再試行する。

## 開発

```bash
nix develop
cargo test
```
