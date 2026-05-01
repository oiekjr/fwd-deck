# CLI Reference

`fwd-deck` の CLIコマンド、設定ファイル、実行状態、JSON出力をまとめます。  
代表例と重要な制約は CLIヘルプにも表示します。

## Global Options

```sh
fwd-deck --config ./my-fwd-deck.toml list
fwd-deck --global-config ~/.config/fwd-deck/work.toml list
fwd-deck --no-global list
fwd-deck --state /tmp/fwd-deck-state.toml status
fwd-deck open ~/projects/my-service
```

| オプション | 説明 |
| --- | --- |
| `--config PATH` | local設定ファイルのパスを指定します。 |
| `--global-config PATH` | global設定ファイルのパスを指定します。 |
| `--no-global` | global設定ファイルを読み込みません。 |
| `--state PATH` | 起動中トンネルの PID などを保存する実行状態ファイルのパスを指定します。 |
| `--json` | 対応コマンドの出力を JSON として標準出力へ表示します。 |

## Configuration Files

既定では以下の2つの設定ファイルを読み込みます。

| 種別 | パス | 用途 |
| --- | --- | --- |
| global | `~/.config/fwd-deck/config.toml` | 複数プロジェクトで共有する設定 |
| local | `./fwd-deck.toml` | 作業ディレクトリ固有の設定 |

同じ `name` は global と local の両方に共存できます。  
bare name を指定する操作では local が優先され、global を対象にする場合は `--scope global` を指定します。  
`fwd-deck.toml` はローカル環境用の設定として git 管理から除外する想定です。

macOSアプリでは、Settings で選択した Workspace を local設定の基準にします。  
アプリの local設定は `<workspace>/fwd-deck.toml` として扱い、最近使った Workspace はアプリ設定として保存します。  
Workspace に `fwd-deck.toml` が存在しない場合でも読み込みは継続し、local への追加時に設定ファイルを作成します。
macOSアプリの設定複製では、複製元とは別に複製先の local設定または global設定を選択できます。
macOSアプリの初回起動時は、アプリ設定 `preferences.toml` が未作成かつ global設定が存在しない場合に、`fwd-deck.example.toml` と同じ内容を global設定へ自動作成します。
この初回起動判定後はアプリ設定を保存し、以後 global設定を削除しても自動再作成しません。

## Configuration Format

```toml
[timeouts]
connect_timeout_seconds = 15
server_alive_interval_seconds = 30
server_alive_count_max = 3
start_grace_milliseconds = 300

[[tunnels]]
name = "dev-db"
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
| `name` | Yes | トンネルを識別する表示名です。 |
| `description` | No | `show` や `list --query` で使う説明です。 |
| `tags` | No | `start --tag` や `list --tag` で使うタグです。 |
| `local_host` | No | ローカル側 bind address です。省略時は `127.0.0.1` です。 |
| `local_port` | Yes | ローカル側ポートです。 |
| `remote_host` | Yes | 転送先ホストです。 |
| `remote_port` | Yes | 転送先ポートです。 |
| `ssh_user` | Yes | SSHユーザーです。 |
| `ssh_host` | Yes | SSH接続先ホストです。 |
| `ssh_port` | No | SSH接続先ポートです。省略時は SSH の既定値を使います。 |
| `identity_file` | No | SSH秘密鍵ファイルです。 |
| `timeouts` | No | トンネル単位のタイムアウト上書きです。 |

`tags` は小文字 ASCII の `a-z`, `0-9`, `-`, `_`, `.`, `/` を使えます。  
同一設定ファイル内の `name` 重複と `local_port` 重複は検証エラーです。  
global と local の間で同じ `name` や `local_port` を使う設定は許容します。  
同じ `local_port` のトンネルを同時に起動しようとした場合、後から起動した側が local endpoint 使用中として失敗します。  
`local_port` が `1024` 未満の場合、`validate` は権限が必要になる可能性を warning として表示します。

### Timeout Fields

`[timeouts]` は全体共通のタイムアウト設定です。  
各 `[[tunnels]]` の `[tunnels.timeouts]` は、そのトンネルだけの上書き設定です。

| フィールド | 既定値 | 説明 |
| --- | --- | --- |
| `connect_timeout_seconds` | `15` | SSH接続のタイムアウト秒数です。 |
| `server_alive_interval_seconds` | `30` | SSH keepalive の送信間隔です。 |
| `server_alive_count_max` | `3` | SSH keepalive の失敗許容回数です。 |
| `start_grace_milliseconds` | `300` | 起動後の疎通確認まで待つ時間です。 |

## Runtime State

起動したトンネルの PID や接続先は、既定で `~/.local/state/fwd-deck/state.toml` に保存します。  
この状態ファイルは `status`, `stop`, `recover`, `watch` が対象プロセスを判断するために使います。  
状態ファイル上では `source_kind`, 正規化した `source_path`, `name` から生成した `runtime_id` で各トンネルを追跡します。

状態ファイルの場所は `--state` で変更できます。

```sh
fwd-deck --state /tmp/fwd-deck-state.toml status
```

macOSアプリの Auto recover / Watch 設定は、アプリ設定の `preferences.toml` に保存します。  
`fwd-deck.toml` と CLIオプションには書き込まず、アプリ常駐中に現在の Workspace と global設定の stale 状態だけを復旧対象にします。

## Command Reference

### `open`

```sh
fwd-deck open
fwd-deck open ~/projects/my-service
```

現在のディレクトリ、または指定した `PATH` を macOSアプリの Workspace として開きます。
`PATH` は既存ディレクトリである必要があります。
相対パスは CLI の現在ディレクトリを基準に解決します。

既存アプリが起動中の場合は、新しいウィンドウを作らずに既存ウィンドウで Workspace を切り替えます。
Workspace 切り替え時は旧 Workspace の local トンネルを停止し、global トンネルは維持します。
旧 Workspace の local トンネル停止に失敗した場合は、Workspace 切り替えを中止します。

macOSアプリが未インストールの場合は、先に Homebrew cask からインストールしてください。

```sh
brew install --cask oiekjr/tap/fwd-deck-app
```

### `list`

```sh
fwd-deck list
fwd-deck --json list
fwd-deck list --wide
fwd-deck list --query db
fwd-deck list --tag dev
fwd-deck list --tag dev --tag project-a
fwd-deck list --tag dev --query db
```

設定済みトンネルを一覧表示します。  
`--query` は `name` と `description` に対して大文字小文字を区別しない部分一致検索を行います。  
`--tag` は複数指定でき、指定したタグをすべて持つトンネルだけを表示します。  
通常の `list` は `REMOTE` の host 部分を省略表示し、`--wide` は `REMOTE` を省略せずに表示します。

### `show`

```sh
fwd-deck show dev-db
fwd-deck --json show dev-db
```

統合後のトンネル詳細を表示します。  
`description`, `tags`, 接続先、有効なタイムアウト設定、読み込み元の設定ファイルを確認できます。  
同じ `name` が local と global の両方にある場合、bare name は local を優先します。  
global を対象にする場合は `--scope global` を指定します。

### `start`

```sh
fwd-deck start
fwd-deck start dev-db
fwd-deck start --all
fwd-deck start --all --parallel 4
fwd-deck start --tag dev --tag project-a
fwd-deck start dev-db --dry-run
fwd-deck start dev-db --scope global --dry-run
fwd-deck --json start dev-db --dry-run
```

設定済みトンネルを起動します。  
NAME を指定しない場合は対話選択を表示します。  
`--all` は設定済みのすべてのトンネルを開始します。  
`--tag` は、指定したタグをすべて持つトンネルだけを開始します。  
`--parallel` は複数トンネルの開始処理を指定件数まで並列実行します。  
`--all`、NAME、`--tag` は同時に指定できません。  
`--dry-run` は SSH を起動せず、状態ファイルも更新せずに実行予定だけを表示します。

同じ `name` が local と global の両方にある場合、bare name は local を優先します。  
global を対象にする場合は `--scope global` を指定します。  
local endpoint が使用中の場合、取得できる範囲でそのポートを使っている LISTENプロセスも表示します。

### `status`

```sh
fwd-deck status
fwd-deck --json status
```

状態ファイル上で追跡しているトンネルの状態を表示します。  
状態ファイルに記録された PID を使い、追跡中トンネルが実行中か stale かを判定します。

### `recover`

```sh
fwd-deck recover
fwd-deck recover dev-db
fwd-deck recover dev-db --scope global
```

stale な追跡中トンネルを現在の設定に基づいて再起動します。  
NAME を省略した場合は、状態ファイルで stale と判定された追跡中トンネルを対象にします。  
NAME を指定した場合は、指定トンネルだけを復旧対象にします。

### `watch`

```sh
fwd-deck watch
fwd-deck watch dev-db --interval-seconds 5
fwd-deck watch dev-db --scope global --interval-seconds 5
```

追跡中のトンネルを監視し、stale になった場合に現在の設定で再起動します。  
NAME を省略した場合は、状態ファイル上の追跡中トンネルを監視します。  
`--interval-seconds` は監視間隔を秒単位で指定します。

### `stop`

```sh
fwd-deck stop
fwd-deck stop dev-db
fwd-deck stop dev-db --scope global
fwd-deck stop --all
fwd-deck stop dev-db --dry-run
```

追跡中トンネルを停止します。  
NAME を指定しない場合は対話選択を表示します。  
`--all` は追跡中のすべてのトンネルを停止します。  
`--all` と NAME は同時に指定できません。  
`--dry-run` は、実際の停止や状態ファイル更新を行わずに実行予定だけを表示します。

同じ `name` が local と global の両方にある場合、bare name は local を優先します。  
global を対象にする場合は `--scope global` を指定します。

### `config`

```sh
fwd-deck config add
fwd-deck config add --scope local
fwd-deck config add --scope global
fwd-deck config edit dev-db
fwd-deck config edit dev-db --scope local
fwd-deck config edit dev-db --scope global
fwd-deck config remove
fwd-deck config remove --scope local
fwd-deck config remove --scope global
```

`config add`, `config edit`, `config remove` は、global設定または local設定を対話形式で編集します。  
`--scope` を省略すると、編集する local または global 設定を対話選択します。  
`--scope local` は `./fwd-deck.toml`、`--scope global` は `~/.config/fwd-deck/config.toml` を対象にします。  
`config add` は対象ファイルが存在しない場合に新規作成します。  
同一設定ファイル内で重複する `name` と `local_port` は入力時に拒否します。  
`config edit` は既存値を初期値として表示し、空入力は既存値維持として扱います。  
同じ `name` が global設定と local設定の両方に存在する場合、対話実行時は編集対象を選択します。  
非対話実行時は `--scope` を指定します。

`config add` はタイムアウト設定を入力しないため、必要な場合は TOML を直接編集します。  
`config edit` もタイムアウト設定を変更しないため、必要な場合は TOML を直接編集します。

### `completion`

```sh
fwd-deck completion zsh
```

シェル補完スクリプトを生成します。  
対応シェルは `bash`, `elvish`, `fish`, `powershell`, `zsh` です。  
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

### `validate`

```sh
fwd-deck validate
fwd-deck --json validate
```

設定ファイルを検証し、エラーと warning を表示します。  
読み込んだ local と global の設定を統合したうえで検証します。

### `doctor`

```sh
fwd-deck doctor
```

設定ファイルの有無、設定検証、状態ファイルの読み書き、`ssh` / `lsof` の起動可否、`identity_file` の存在、local endpoint の使用状況をまとめて確認します。

## JSON Output

`--json` は `list`, `show`, `status`, `validate`, `start --dry-run` で利用できます。  
通常のメッセージではなく機械処理向けの JSON を標準出力へ表示します。
