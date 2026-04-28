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
fwd-deck validate
```

開発中は以下のように実行できます。

```sh
cargo run -p fwd-deck-cli --bin fwd-deck -- list
cargo run -p fwd-deck-cli --bin fwd-deck -- status
cargo run -p fwd-deck-cli --bin fwd-deck -- validate
```

`start` と `stop` は、ID を指定しない場合に対話選択を表示します。
`stop` の対話選択には、追跡中のトンネルをまとめて停止する選択肢も表示されます。
`recover` は、状態ファイル上で stale になっているトンネルを現在の設定に基づいて再起動します。

## 設定ファイル

既定では以下の 2 つを読み込みます。

- グローバル設定: `~/.config/fwd-deck/config.toml`
- ローカル設定: `./fwd-deck.toml`

同じ `id` がある場合は、ローカル設定がグローバル設定を上書きします。
`local_host` を省略した場合は `127.0.0.1` として扱います。
`fwd-deck.toml` はローカル環境用の設定として git 管理から除外しています。

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
task list
task recover
task status
task validate
```

`task` が直接見つからない環境では、mise 経由で実行できます。

```sh
mise exec -- task check
```
