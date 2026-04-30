# Configuration Reference

`fwd-deck` の設定ファイルの場所、TOML例、各フィールド、ランタイム状態の保存先をまとめます。  

## Configuration Files

既定では以下の2つの設定ファイルを読み込みます。  

| 種別 | パス | 用途 |
| --- | --- | --- |
| グローバル設定 | `~/.config/fwd-deck/config.toml` | 複数プロジェクトで共有する設定 |
| ローカル設定 | `./fwd-deck.toml` | 作業ディレクトリ固有の設定 |

同じ `name` は global と local の両方に共存できます。  
bare name を指定する操作では local が優先され、global を対象にする場合は `--scope global` を指定します。  
`fwd-deck.toml` はローカル環境用の設定として git 管理から除外する想定です。  

設定ファイルの場所は CLIオプションで変更できます。  

```sh
fwd-deck --config ./my-fwd-deck.toml list
fwd-deck --global-config ~/.config/fwd-deck/work.toml list
fwd-deck --no-global list
```

macOSアプリでは、CLI の実行ディレクトリではなく、Settings で選択したワークスペースディレクトリを local設定の基準にします。  
アプリの local設定は `<workspace>/fwd-deck.toml` として扱い、最近使ったワークスペースはアプリ設定として保存します。  
ワークスペースに `fwd-deck.toml` が存在しない場合でも読み込みは継続し、local への追加時に設定ファイルを作成します。  

## Example

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

## Tunnel Fields

| フィールド | 必須 | 説明 |
| --- | --- | --- |
| `name` | Yes | トンネルを識別する表示名 |
| `description` | No | `show` や `list --query` で使う説明 |
| `tags` | No | `start --tag` や `list --tag` で使うタグ |
| `local_host` | No | ローカル側bind address、省略時は `127.0.0.1` |
| `local_port` | Yes | ローカル側ポート |
| `remote_host` | Yes | 転送先ホスト |
| `remote_port` | Yes | 転送先ポート |
| `ssh_user` | Yes | SSHユーザー |
| `ssh_host` | Yes | SSH接続先ホスト |
| `ssh_port` | No | SSH接続先ポート、省略時は SSH の既定値を使用 |
| `identity_file` | No | SSH秘密鍵ファイル |
| `timeouts` | No | トンネル単位のタイムアウト上書き |

`tags` は小文字 ASCII の `a-z`, `0-9`, `-`, `_`, `.`, `/` を使えます。  
同一設定ファイル内の `name` 重複と `local_port` 重複は検証エラーです。  
global と local の間で同じ `name` や `local_port` を使う設定は許容します。  
同じ `local_port` のトンネルを同時に起動しようとした場合、後から起動した側が local endpoint 使用中として失敗します。  
`local_port` が `1024` 未満の場合、`validate` は権限が必要になる可能性を warning として表示します。  

## Timeout Settings

`[timeouts]` は全体共通のタイムアウト設定です。  
各 `[[tunnels]]` の `[tunnels.timeouts]` は、そのトンネルだけの上書き設定です。  

| フィールド | 既定値 | 説明 |
| --- | --- | --- |
| `connect_timeout_seconds` | `15` | SSH接続のタイムアウト秒数 |
| `server_alive_interval_seconds` | `30` | SSH keepalive の送信間隔 |
| `server_alive_count_max` | `3` | SSH keepalive の失敗許容回数 |
| `start_grace_milliseconds` | `300` | 起動後の疎通確認まで待つ時間 |

## Runtime State

起動したトンネルの PID や接続先は、既定で `~/.local/state/fwd-deck/state.toml` に保存します。  
この状態ファイルは `status`, `stop`, `recover`, `watch` が対象プロセスを判断するために使います。  
状態ファイル上では `source_kind`, 正規化した `source_path`, `name` から生成した `runtime_id` で各トンネルを追跡します。  

macOSアプリの Auto recover / Watch 設定は、アプリ設定の `preferences.toml` に保存します。  
`fwd-deck.toml` と CLIオプションには書き込まず、アプリ常駐中に現在の Workspace と global 設定の stale 状態だけを復旧対象にします。  
復旧直後に再度 stale になった場合は失敗として扱い、段階的なバックオフを入れて初回失敗だけを通知します。  

状態ファイルの場所は `--state` で変更できます。  

```sh
fwd-deck --state /tmp/fwd-deck-state.toml status
```
