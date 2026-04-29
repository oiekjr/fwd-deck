# Development

依存ツールの導入、よく使う task、ローカル実行、品質確認の手順をまとめます。  

## Setup

開発用の Rust と go-task は `mise.toml` で管理しています。  

```sh
mise install
```

macOSアプリのフロントエンド依存関係をインストールします。  

```sh
task app:install
```

## Run Development Commands

よく使う開発コマンドは `Taskfile.yml` に定義しています。  

```sh
task --list
task check
task fmt
task test
task lint
```

macOSアプリ Fwd Deck は `apps/fwd-deck-app` にあります。  
Tauri の Rust command層は `fwd-deck-core` を直接利用し、フロントエンドは React / Vite / TypeScript で構成しています。  

```sh
task app:dev
task app:build:web
task app:lint
task app:format:check
task app:check
```

## Run CLI Locally

ローカルの `fwd-deck.toml` を使って CLI の動作を確認する場合は、次の task を使います。  

```sh
task validate
task list
task start:dry-run
task status
task stop:dry-run
task watch
```

task を介さずに CLI を直接起動したい場合は、開発用の補助として `cargo run` を使えます。  

```sh
cargo run -p fwd-deck-cli --bin fwd-deck -- --help
cargo run -p fwd-deck-cli --bin fwd-deck -- list
```

## Check Before Completion

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

リリース手順は [Release](release.md) を参照してください。  
