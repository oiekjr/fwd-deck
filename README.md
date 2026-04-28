# fwd-deck

設定に定義したローカルポートフォワーディングを、CLI から操作するためのツールです。

## コマンド

```sh
fwd-deck list
fwd-deck start
fwd-deck start dev-db
fwd-deck recover
fwd-deck recover dev-db
fwd-deck status
fwd-deck stop
fwd-deck stop dev-db
fwd-deck config add
fwd-deck config add --scope local
fwd-deck config remove
fwd-deck config remove --scope global
fwd-deck validate
```

開発中は以下のように実行できます。

```sh
cargo run -p fwd-deck-cli --bin fwd-deck -- list
cargo run -p fwd-deck-cli --bin fwd-deck -- status
cargo run -p fwd-deck-cli --bin fwd-deck -- config add
cargo run -p fwd-deck-cli --bin fwd-deck -- validate
```

`start` と `stop` は、ID を指定しない場合に対話選択を表示します。
`stop` の対話選択には、追跡中のトンネルをまとめて停止する選択肢も表示されます。
`recover` は、状態ファイル上で stale になっているトンネルを現在の設定に基づいて再起動します。
`config add` と `config remove` は、グローバル設定またはローカル設定を選択して対話形式で編集します。

## 設定ファイル

既定では以下の 2 つを読み込みます。

- グローバル設定: `~/.config/fwd-deck/config.toml`
- ローカル設定: `./fwd-deck.toml`

同じ `id` がある場合は、ローカル設定がグローバル設定を上書きします。
`local_host` を省略した場合は `127.0.0.1` として扱います。
`local_port` が `1024` 未満の場合、`validate` は権限が必要になる可能性を warning として表示します。
`fwd-deck.toml` はローカル環境用の設定として git 管理から除外しています。
`config add` は対象ファイルが存在しない場合に新規作成します。
`config remove` は対象ファイル内のトンネルだけを削除対象として表示します。

```sh
cp fwd-deck.example.toml fwd-deck.toml
```

```toml
[[tunnels]]
id = "dev-db"
description = "Development database"
local_host = "127.0.0.1"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
ssh_port = 22
identity_file = "~/.ssh/id_ed25519"
```

## 状態ファイル

起動したトンネルの PID や接続先は、既定で `~/.local/state/fwd-deck/state.toml` に保存します。
この状態ファイルは `status` と `stop` が対象プロセスを判断するために使います。

## 開発

よく使う開発コマンドは `Taskfile.yml` に定義しています。

```sh
mise install
task --list
task check
task config:add
task config:remove
task list
task recover
task status
task validate
```
