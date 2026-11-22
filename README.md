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

Nix を使わない場合は、[リリース](https://github.com/shiroinock/dept/releases)からバイナリを取れる
(`aarch64-apple-darwin`、`x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`)。
バイナリには GitHub Actions でビルドした来歴(SLSA build provenance)と SBOM(CycloneDX)の
attestation が付いているので、使う前に確かめられる。

```bash
gh release download -R shiroinock/dept -p 'dept-aarch64-apple-darwin.tar.gz'
gh attestation verify dept-aarch64-apple-darwin.tar.gz -R shiroinock/dept
tar -xzf dept-aarch64-apple-darwin.tar.gz dept
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
| `[PATH]` | ロックファイルか、それを含むディレクトリ(既定は `.`) |
| `--weights 30,3,1` | 重み(MAJOR,MINOR,PATCH) |
| `--prod` | devDependencies を除く |
| `--json` | JSON で出力する(CI で保存して推移を見る用) |
| `--all` / `--top N` | 最新の依存も表示する / 表示件数を絞る |
| `--fail-over N` | 合計スコアが N を超えたら終了コード 2 |
| `--badge FILE` | README に貼るバッジ(SVG)を書く |
| `--badge-label dept` | バッジの左側の文字 |
| `--badge-colors 100,500` | バッジの色の境目(GREEN,YELLOW)。GREEN 以下は緑、YELLOW 以下は黄、それより上は赤 |
| `--concurrency 8` | レジストリへの同時リクエスト数 |
| `--cache-ttl 12` | キャッシュの有効期間(時間) |
| `--offline` / `--no-cache` | キャッシュだけを使う / キャッシュを使わない |

## README のバッジ

GitHub Actions で dept を定期的に走らせ、スコアのバッジを README に出せる。
バッジの SVG は dept が自分で作り、shields.io などの外部サービスは使わない。

```yaml
# .github/workflows/dept.yml
name: dept

on:
  push:
    branches: [main]
  schedule:
    - cron: "0 0 * * 1" # 毎週月曜
  workflow_dispatch:

permissions: {}

# 近い時刻に 2 つ走っても、古い結果があとから上書きしないようにする
concurrency:
  group: dept
  cancel-in-progress: true

jobs:
  dept:
    runs-on: ubuntu-latest
    permissions:
      contents: write # バッジのブランチに push する
    steps:
      - uses: actions/checkout@<sha> # v7
        with:
          persist-credentials: false
      - uses: shiroinock/dept@<sha> # v0.2.0
        # with:
        #   path: .              # ロックファイルか、それを含むディレクトリ
        #   args: --prod         # dept に渡す追加の引数
        #   allow-errors: false  # 調べられなかった依存があっても置くか
        #   branch: dept-badge   # 置き先。空にすると push しない
```

```markdown
![dept](https://raw.githubusercontent.com/<owner>/<repo>/dept-badge/dept.svg)
```

- アクションは、自分と同じバージョンの dept をリリースから取る。来歴の attestation を確かめてから使う。
  タグを付けたコミット(リリース)に固定して使う。
- `dept-badge` ブランチに `dept.svg` と `dept.json` を置く。ブランチは毎回 1 コミットに置き換え、中身が変わらなければ push しない。
- `raw.githubusercontent.com` を使うので、公開リポジトリ向け。
- 調べられなかった依存(レジストリのエラーなど)があると、スコアが低く出るので、push せずに失敗にする。
  見つからない非公開パッケージなどがあって構わないなら `allow-errors: true` にする。
- `args` に `--fail-over N` を渡すと、バッジを置いたあとでジョブを失敗にする。
- 出力:`score`(合計スコア)、`dir`(`dept.svg` と `dept.json` を置いたディレクトリ)。
- 対応するランナーは Linux(x64 / ARM64)と macOS(ARM64)。

## 対応状況

| ロックファイル | 対象 |
| --- | --- |
| `pnpm-lock.yaml`(lockfileVersion 6 / 9) | 各 importer(ワークスペースの各パッケージ)の直接依存 |
| `package-lock.json`(lockfileVersion 2 / 3) | ルートパッケージの直接依存 |

- ディレクトリを渡すと、上のロックファイルを探す。複数あるときはファイルを指定する。
- ワークスペース内のパッケージ(`link:`)、`file:`・tarball・git から入れている依存は対象外。
- エイリアス(`npm:other@^1`)は実際のパッケージ名で調べる。
- モノレポで同じパッケージの同じバージョンが複数の importer にあれば 1 行にまとめる。

### バージョン情報の取り先

サードパーティの API は使わず、パッケージマネージャーが使うレジストリから直接取る。

- npm / pnpm:npm レジストリ(既定は `https://registry.npmjs.org/`)。
  `.npmrc` の `registry=` と `@scope:registry=` に従う(優先順は 環境変数 `npm_config_registry` > プロジェクトの `.npmrc` > `~/.npmrc`)。
  認証が要るレジストリにはまだ対応していない。

取った結果は、必要なフィールドだけを `$XDG_CACHE_HOME/dept/`(未設定なら `~/.cache/dept/`)にキャッシュする。
429 / 5xx と接続エラーは指数バックオフで再試行する。

## 開発

```bash
nix develop
cargo test
```

### リリース

1. `Cargo.toml` の `version` を上げる PR をマージする。
2. main の先頭に `v<version>` のタグを付けて push する。

   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

3. `release.yml` が、タグと `Cargo.toml` の version が同じかを確かめてから、各ターゲットをビルドする。
   tar.gz、`SHA256SUMS`、SBOM(`dept.cdx.json`)をリリースに載せ、来歴と SBOM の attestation を付ける。

`v*` タグはルールセットで削除と付け替えを禁じ、リリースは immutable にしている(公開後にアセットを差し替えられない)。
