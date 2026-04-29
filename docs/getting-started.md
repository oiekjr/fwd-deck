# Getting Started

最小構成の設定ファイルを作成し、トンネルの確認、起動、停止までを順番に行います。  

## Install

`fwd-deck` は Homebrew tap からインストールできます。  
SSH接続には OpenSSH client を使用します。  

```sh
brew install oiekjr/tap/fwd-deck
brew install --cask oiekjr/tap/fwd-deck-app
```

tap を追加済みの場合は、短い名前でインストールできます。  

```sh
brew install fwd-deck
brew install --cask fwd-deck-app
```

macOSアプリは当面、個人利用向けの unsigned app として配布します。  
Gatekeeper で止まる場合は Finder で右クリックして Open を選択するか、Homebrew の `--no-quarantine` option を付けてインストールします。  

インストール後、CLI が利用できることを確認します。  

```sh
fwd-deck --help
```

Rust toolchain がある環境では、開発用に `cargo` から直接インストールできます。  

```sh
cargo install --git https://github.com/oiekjr/fwd-deck.git fwd-deck-cli
```

更新する場合は `--force` を付けて再インストールします。  

```sh
cargo install --git https://github.com/oiekjr/fwd-deck.git fwd-deck-cli --force
```

## Create Configuration

まず設定ファイルを作成します。  

```sh
cp fwd-deck.example.toml fwd-deck.toml
```

`fwd-deck.toml` を自分の SSH接続先に合わせて編集します。  
設定項目の詳細は [Configuration Reference](configuration.md) を参照してください。  

## Validate Configuration

設定を検証します。  

```sh
fwd-deck validate
```

エラーが表示された場合は、対象のフィールドを修正してから再度検証します。  

## List Tunnels

登録済みのトンネルを一覧表示します。  

```sh
fwd-deck list
```

`description` を含む詳細を確認する場合は `show` を使います。  

```sh
fwd-deck show dev-db
```

## Start And Stop

実際に SSH を起動する前に、実行予定を確認します。  

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

CLIコマンドの一覧は [CLI Reference](cli.md) を参照してください。  
