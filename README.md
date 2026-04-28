# fwd-deck

設定に定義したローカルポートフォワーディングを、CLI から操作するためのツールです。

## コマンド

```sh
fwd-deck list
fwd-deck validate
```

開発中は以下のように実行できます。

```sh
cargo run -p fwd-deck-cli --bin fwd-deck -- list
cargo run -p fwd-deck-cli --bin fwd-deck -- validate
```

## 設定ファイル

既定では以下の 2 つを読み込みます。

- グローバル設定: `~/.config/fwd-deck/config.toml`
- ローカル設定: `./fwd-deck.toml`

同じ `id` がある場合は、ローカル設定がグローバル設定を上書きします。
`local_host` を省略した場合は `127.0.0.1` として扱います。

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

## 開発

よく使う開発コマンドは `Taskfile.yml` に定義しています。

```sh
mise install
task --list
task check
task list
task validate
```

`task` が直接見つからない環境では、mise 経由で実行できます。

```sh
mise exec -- task check
```
