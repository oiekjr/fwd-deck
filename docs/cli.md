# CLI Reference

`fwd-deck` の CLIコマンド、代表的な呼び出し、主要オプションの意味をまとめます。  

## List And Search

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

## Show Details

```sh
fwd-deck show dev-db
```

`show` は、統合後のトンネル詳細を表示します。  
`description`, `tags`, 接続先、有効なタイムアウト設定、読み込み元の設定ファイルを確認できます。  

## Start Tunnels

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

local endpoint が使用中の場合、取得できる範囲でそのポートを使っている LISTENプロセスも表示します。  

## Status And Recovery

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

## Stop Tunnels

```sh
fwd-deck stop
fwd-deck stop dev-db
fwd-deck stop --all
fwd-deck stop dev-db --dry-run
```

`stop` は ID を指定しない場合に対話選択を表示します。  
`stop --all` は追跡中のすべてのトンネルを停止します。  
`stop --dry-run` は、実際の停止や状態ファイル更新を行わずに実行予定だけを表示します。  

## Edit Configuration

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

## Shell Completion

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

## Validate

```sh
fwd-deck validate
```

`validate` は設定ファイルを検証し、エラーと warning を表示します。  
