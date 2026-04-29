# fwd-deck

`fwd-deck` は、設定ファイルに定義したローカルポートフォワーディングを CLI から操作するためのツールです。
複数の SSH トンネルを `id` や `tag` で管理し、起動、停止、状態確認、自動回復をまとめて扱えます。

## Installation

`fwd-deck` は Rust toolchain の `cargo` からインストールできます。
SSH 接続には OpenSSH client を使用します。

```sh
cargo install --git https://github.com/oiekjr/fwd-deck.git fwd-deck-cli
```

インストール後、次のコマンドで CLI が利用できることを確認します。

```sh
fwd-deck --help
```

更新する場合は `--force` を付けて再インストールします。

```sh
cargo install --git https://github.com/oiekjr/fwd-deck.git fwd-deck-cli --force
```

## Quick Start

まず設定ファイルを作成します。

```sh
cp fwd-deck.example.toml fwd-deck.toml
```

`fwd-deck.toml` を自分の SSH 接続先に合わせて編集し、設定を検証します。

```sh
fwd-deck validate
```

登録済みのトンネルを一覧表示します。

```sh
fwd-deck list
```

`description` を含む詳細を確認する場合は `show` を使います。

```sh
fwd-deck show dev-db
```

実際に SSH を起動する前に、実行予定を確認できます。

```sh
fwd-deck start dev-db --dry-run
```

トンネルを起動し、状態を確認します。

```sh
fwd-deck start dev-db
fwd-deck status
```

停止する場合は `stop` を使います。

```sh
fwd-deck stop dev-db
```

## Configuration

既定では以下の 2 つの設定ファイルを読み込みます。

| 種別 | パス | 用途 |
| --- | --- | --- |
| グローバル設定 | `~/.config/fwd-deck/config.toml` | 複数プロジェクトで共有する設定 |
| ローカル設定 | `./fwd-deck.toml` | 作業ディレクトリ固有の設定 |

同じ `id` がある場合は、ローカル設定がグローバル設定を上書きします。
`fwd-deck.toml` はローカル環境用の設定として git 管理から除外する想定です。

設定ファイルの場所は CLI オプションで変更できます。

```sh
fwd-deck --config ./my-fwd-deck.toml list
fwd-deck --global-config ~/.config/fwd-deck/work.toml list
fwd-deck --no-global list
```

macOS アプリでは、CLI の実行ディレクトリではなく、Settings で選択したワークスペースディレクトリを local 設定の基準にします。
アプリの local 設定は `<workspace>/fwd-deck.toml` として扱い、最近使ったワークスペースはアプリ設定として保存します。
ワークスペースに `fwd-deck.toml` が存在しない場合でも読み込みは継続し、local への追加時に設定ファイルを作成します。

### Example

```toml
[timeouts]
connect_timeout_seconds = 15
server_alive_interval_seconds = 30
server_alive_count_max = 3
start_grace_milliseconds = 300

[[tunnels]]
id = "dev-db"
description = "Development database"
tags = ["dev", "project-a"]
local_host = "127.0.0.1"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
ssh_port = 22
identity_file = "~/.ssh/id_ed25519"

[tunnels.timeouts]
connect_timeout_seconds = 10
```

### Tunnel Fields

| フィールド | 必須 | 説明 |
| --- | --- | --- |
| `id` | Yes | トンネルを識別する名前 |
| `description` | No | `show` や `list --query` で使う説明 |
| `tags` | No | `start --tag` や `list --tag` で使うタグ |
| `local_host` | No | ローカル側 bind address。省略時は `127.0.0.1` |
| `local_port` | Yes | ローカル側 port |
| `remote_host` | Yes | 転送先 host |
| `remote_port` | Yes | 転送先 port |
| `ssh_user` | Yes | SSH ユーザー |
| `ssh_host` | Yes | SSH 接続先 host |
| `ssh_port` | No | SSH 接続先 port。省略時は SSH の既定値を使用 |
| `identity_file` | No | SSH 秘密鍵ファイル |
| `timeouts` | No | トンネル単位のタイムアウト上書き |

`tags` は小文字 ASCII の `a-z`, `0-9`, `-`, `_`, `.`, `/` を使えます。
`local_port` が `1024` 未満の場合、`validate` は権限が必要になる可能性を warning として表示します。

### Timeout Settings

`[timeouts]` は全体共通のタイムアウト設定です。
各 `[[tunnels]]` の `[tunnels.timeouts]` は、そのトンネルだけの上書き設定です。

| フィールド | 既定値 | 説明 |
| --- | --- | --- |
| `connect_timeout_seconds` | `15` | SSH 接続のタイムアウト秒数 |
| `server_alive_interval_seconds` | `30` | SSH keepalive の送信間隔 |
| `server_alive_count_max` | `3` | SSH keepalive の失敗許容回数 |
| `start_grace_milliseconds` | `300` | 起動後の疎通確認まで待つ時間 |

### Runtime State

起動したトンネルの PID や接続先は、既定で `~/.local/state/fwd-deck/state.toml` に保存します。
この状態ファイルは `status`, `stop`, `recover`, `watch` が対象プロセスを判断するために使います。

状態ファイルの場所は `--state` で変更できます。

```sh
fwd-deck --state /tmp/fwd-deck-state.toml status
```

## Usage

### List And Search

```sh
fwd-deck list
fwd-deck list --wide
fwd-deck list --query db
fwd-deck list --tag dev
fwd-deck list --tag dev --query db
```

`list --query` は、`id` と `description` に対して大文字小文字を区別しない部分一致検索を行います。
通常の `list` は `REMOTE` の host 部分を省略表示し、`--wide` は `REMOTE` を省略せずに表示します。
`--tag` は複数指定でき、指定したタグをすべて持つトンネルだけを表示します。
`--tag` と `--query` を併用した場合は、両方に一致するトンネルだけを表示します。

### Show Details

```sh
fwd-deck show dev-db
```

`show` は、統合後のトンネル詳細を表示します。
`description`, `tags`, 接続先、有効なタイムアウト設定、読み込み元の設定ファイルを確認できます。

### Start Tunnels

```sh
fwd-deck start
fwd-deck start dev-db
fwd-deck start --all
fwd-deck start --tag dev --tag project-a
fwd-deck start dev-db --dry-run
```

`start` は ID を指定しない場合に対話選択を表示します。
`start --all` は設定済みのすべてのトンネルを開始します。
`start --tag` は、指定したタグをすべて持つトンネルだけを開始します。
`start --dry-run` は、実際の起動や状態ファイル更新を行わずに実行予定だけを表示します。

local endpoint が使用中の場合、取得できる範囲でそのポートを使っている LISTEN プロセスも表示します。

### Status And Recovery

```sh
fwd-deck status
fwd-deck recover
fwd-deck recover dev-db
fwd-deck watch
fwd-deck watch dev-db --interval-seconds 5
```

`status` は状態ファイル上で追跡しているトンネルの状態を表示します。
`recover` は stale になっているトンネルを現在の設定に基づいて再起動します。
`watch` は追跡中のトンネルを監視し、stale になった場合に自動で再起動します。

### Stop Tunnels

```sh
fwd-deck stop
fwd-deck stop dev-db
fwd-deck stop --all
fwd-deck stop dev-db --dry-run
```

`stop` は ID を指定しない場合に対話選択を表示します。
`stop --all` は追跡中のすべてのトンネルを停止します。
`stop --dry-run` は、実際の停止や状態ファイル更新を行わずに実行予定だけを表示します。

### Edit Configuration

```sh
fwd-deck config add
fwd-deck config add --scope local
fwd-deck config add --scope global
fwd-deck config remove
fwd-deck config remove --scope local
fwd-deck config remove --scope global
```

`config add` と `config remove` は、グローバル設定またはローカル設定を対話形式で編集します。
`config add` は対象ファイルが存在しない場合に新規作成します。
既存の有効設定と重複する `id` と `local_port` は入力時に拒否します。

`config add` はタイムアウト設定を入力しないため、必要な場合は TOML を直接編集します。

### Shell Completion

```sh
fwd-deck completion zsh
```

zsh で補完を有効にする場合は、生成した補完スクリプトを `fpath` 配下へ配置します。

```sh
mkdir -p ~/.zfunc
fwd-deck completion zsh > ~/.zfunc/_fwd-deck
```

`~/.zshrc` で `~/.zfunc` を `fpath` に追加し、`compinit` を有効にします。

```sh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit
compinit
```

### Validate

```sh
fwd-deck validate
```

`validate` は設定ファイルを検証し、エラーと warning を表示します。

## Development

開発用の Rust と go-task は `mise.toml` で管理しています。

```sh
mise install
```

よく使う開発コマンドは `Taskfile.yml` に定義しています。

```sh
task --list
task check
task fmt
task test
task lint
```

macOS アプリ Fwd Deck は `apps/fwd-deck-app` にあります。
Tauri の Rust command 層は `fwd-deck-core` を直接利用し、フロントエンドは React / Vite / TypeScript で構成しています。

```sh
task app:install
task app:dev
task app:build:web
task app:lint
task app:format:check
task app:check
```

ローカルの `fwd-deck.toml` を使って CLI の動作を確認する場合は、次の task を使います。

```sh
task validate
task list
task start:dry-run
task status
task stop:dry-run
task watch
```

`task check` は整形確認、テスト、clippy をまとめて実行します。
CI でも同じ品質確認を行います。

```sh
cargo fmt --all --check
npm --prefix apps/fwd-deck-app run format:check
npm --prefix apps/fwd-deck-app run build
npm --prefix apps/fwd-deck-app run lint
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

task を介さずに CLI を直接起動したい場合は、開発用の補助として `cargo run` を使えます。

```sh
cargo run -p fwd-deck-cli --bin fwd-deck -- --help
cargo run -p fwd-deck-cli --bin fwd-deck -- list
```

ワークスペースは CLI と core crate に分かれています。

```text
apps/fwd-deck-app   Tauri app and React frontend
crates/fwd-deck-cli   CLI entrypoint and user interaction
crates/fwd-deck-core  Configuration, state, and tunnel runtime logic
```
