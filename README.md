# fwd-deck

`fwd-deck` は、設定ファイルに定義したローカルポートフォワーディングを CLI と macOSアプリから操作するためのツールです。  
複数の SSHトンネルを `id` や `tag` で管理し、起動、停止、状態確認、自動回復をまとめて扱えます。  

## Documentation

目的別の詳しい情報は、次の文書に分けています。  

| 目的 | 文書 |
| --- | --- |
| 初めて使う | [Getting Started](docs/getting-started.md) |
| 設定項目を確認する | [Configuration Reference](docs/configuration.md) |
| CLIコマンドを確認する | [CLI Reference](docs/cli.md) |
| 開発環境を使う | [Development](docs/development.md) |
| リリースする | [Release](docs/release.md) |

## Install

`fwd-deck` は Homebrew tap からインストールできます。  

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
Gatekeeper で止まる場合は Finder で右クリックして Open を選択します。  

## Quick Example

```sh
cp fwd-deck.example.toml fwd-deck.toml
fwd-deck validate
fwd-deck doctor
fwd-deck list
fwd-deck start dev-db --dry-run
fwd-deck start dev-db
fwd-deck status
fwd-deck stop dev-db
```

操作の流れは [Getting Started](docs/getting-started.md) を参照してください。  

## Project Layout

```text
apps/fwd-deck-app      Tauri app and React frontend
crates/fwd-deck-cli    CLI entrypoint and user interaction
crates/fwd-deck-core   Configuration, state, and tunnel runtime logic
```

## License

MIT License で公開しています。  
詳細は [LICENSE](LICENSE) を参照してください。  
