# CLI Reference

`fwd-deck` の CLIコマンド、代表的な呼び出し、主要オプションの意味をまとめます。  
CLIヘルプにも同じ代表例と重要な制約を表示します。  

## Common Options

```sh
fwd-deck --config ./my-fwd-deck.toml list
fwd-deck --global-config ~/.config/fwd-deck/work.toml list
fwd-deck --no-global list
fwd-deck --state /tmp/fwd-deck-state.toml status
```

`--config` は local設定ファイルのパスを指定します。  
`--global-config` は global設定ファイルのパスを指定します。  
`--no-global` は global設定ファイルを読み込まない場合に使います。  
`--state` は起動中トンネルのPIDなどを保存する実行状態ファイルのパスを指定します。  

## List And Search

```sh
fwd-deck list
fwd-deck --json list
fwd-deck list --wide
fwd-deck list --query db
fwd-deck list --tag dev
fwd-deck list --tag dev --tag project-a
fwd-deck list --tag dev --query db
```

`list --query` は、`id` と `description` に対して大文字小文字を区別しない部分一致検索を行います。  
通常の `list` は `REMOTE` の host 部分を省略表示し、`--wide` は `REMOTE` を省略せずに表示します。  
`--tag` は複数指定でき、指定したタグをすべて持つトンネルだけを表示します。  
`--tag` と `--query` を併用した場合は、両方に一致するトンネルだけを表示します。  

## Show Details

```sh
fwd-deck show dev-db
fwd-deck --json show dev-db
```

`show` は、統合後のトンネル詳細を表示します。  
`description`, `tags`, 接続先、有効なタイムアウト設定、読み込み元の設定ファイルを確認できます。  

## Start Tunnels

```sh
fwd-deck start
fwd-deck start dev-db
fwd-deck start --all
fwd-deck start --all --parallel 4
fwd-deck start --tag dev --tag project-a
fwd-deck start dev-db --dry-run
fwd-deck --json start dev-db --dry-run
```

`start` は ID を指定しない場合に対話選択を表示します。  
`start --all` は設定済みのすべてのトンネルを開始します。  
`start --parallel` は複数トンネルの開始処理を指定件数まで並列実行します。  
`start --tag` は、指定したタグをすべて持つトンネルだけを開始します。  
`--all`、ID、`--tag` は同時に指定できません。  
`start --dry-run` は、SSH を起動せず、状態ファイルも更新せずに実行予定だけを表示します。  

local endpoint が使用中の場合、取得できる範囲でそのポートを使っている LISTENプロセスも表示します。  

## Status And Recovery

```sh
fwd-deck status
fwd-deck --json status
fwd-deck recover
fwd-deck recover dev-db
fwd-deck watch
fwd-deck watch dev-db --interval-seconds 5
```

`status` は状態ファイル上で追跡しているトンネルの状態を表示します。  
`recover` は、ID を省略した場合に状態ファイルで stale と判定された追跡中トンネルを現在の設定に基づいて再起動します。  
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
`--all` と ID は同時に指定できません。  
`stop --dry-run` は、実際の停止や状態ファイル更新を行わずに実行予定だけを表示します。  

## Edit Configuration

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

`config add`, `config edit`, `config remove` は、グローバル設定またはローカル設定を対話形式で編集します。  
`--scope` を省略すると、編集する local または global 設定を対話選択します。  
`--scope local` は `./fwd-deck.toml`、`--scope global` は `~/.config/fwd-deck/config.toml` を対象にします。  
`config add` は対象ファイルが存在しない場合に新規作成します。  
既存の有効設定と重複する `id` と `local_port` は入力時に拒否します。  
`config edit` は既存値を初期値として表示し、空入力は既存値維持として扱います。  
同じ `id` がグローバル設定とローカル設定の両方に存在する場合、対話実行時は編集対象を選択します。  
非対話実行時は `--scope` を指定します。  

`config add` はタイムアウト設定を入力しないため、必要な場合は TOML を直接編集します。  
`config edit` もタイムアウト設定を変更しないため、必要な場合は TOML を直接編集します。  

## Shell Completion

```sh
fwd-deck completion zsh
```

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

## Validate

```sh
fwd-deck validate
fwd-deck --json validate
```

`validate` は設定ファイルを検証し、エラーと warning を表示します。  

## Doctor

```sh
fwd-deck doctor
```

`doctor` は設定ファイルの有無、設定検証、状態ファイルの読み書き、`ssh` / `lsof` の起動可否、`identity_file` の存在、local endpoint の使用状況をまとめて確認します。  

## JSON Output

`--json` は `list`, `show`, `status`, `validate`, `start --dry-run` で利用できます。  
通常のメッセージではなく機械処理向けの JSON を標準出力へ表示します。  
