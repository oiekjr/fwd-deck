# Release

配布先は汎用 tap の `oiekjr/homebrew-tap` とし、release tag から GitHub Releases と Homebrew tap を更新します。  

## One-Time Setup

初回だけ、Homebrew tapリポジトリと GitHub Actions secret を手動で準備します。  
この作業が終わると、以後の release tag push で formula と cask を自動更新できます。  

### Prepare Public Repository

Homebrew tap から通常の `brew install` で配布するため、本体リポジトリ `oiekjr/fwd-deck` は public にします。  
release workflow は GitHub tag archive の SHA256 を算出し、formula と cask は GitHub Releases の asset を参照します。  
本体リポジトリが private のままだと、これらの URL が Homebrew から取得できません。  

public 化前に、少なくとも次を確認します。  

1. repo直下に `LICENSE` がある
2. tracked file に secret、token、credential、個人用設定が含まれていない
3. `fwd-deck.toml` などのローカル設定ファイルが git 管理されていない
4. GitHub Releases で公開される DMG が unsigned app であることを README と release手順に明記している

### Create Tap Repository

GitHub で publicリポジトリ `oiekjr/homebrew-tap` を作成します。  
License は本体リポジトリに合わせて MIT License を選択します。  

Homebrew の tap名は `oiekjr/tap` です。  
GitHubリポジトリ名は `homebrew-` prefix を含む `homebrew-tap` にします。  

ローカルに tap の初期構成を作成します。  

```sh
brew tap-new oiekjr/homebrew-tap
```

作成された tap は、Homebrew上では `oiekjr/tap` として参照します。  
次のコマンドで作業ディレクトリへ移動します。  

```sh
cd "$(brew --repository oiekjr/tap)"
```

GitHubリポジトリが空の場合は、remote を追加してそのまま push します。  

```sh
git remote add origin git@github.com:oiekjr/homebrew-tap.git
git push -u origin main
```

GitHubリポジトリ作成時に `LICENSE` や `README.md` を追加した場合、remote側に初期 commit があります。  
その場合は remote側を取り込んでから push します。  

```sh
git remote add origin git@github.com:oiekjr/homebrew-tap.git
git fetch origin main
git merge --allow-unrelated-histories origin/main
git push -u origin main
```

conflict が出た場合は、MIT License の `LICENSE` を残し、`README.md` は tap用の内容へ統合します。  
解決後に commit してから push します。  

```sh
git add .
git commit
git push -u origin main
```

### Create Tap Update Token

本体リポジトリの release workflow から `oiekjr/homebrew-tap` に push するため、fine-grained personal access token を作成します。  

推奨設定は次のとおりです。  

| 項目 | 値 |
| --- | --- |
| Token name | `fwd-deck-homebrew-tap-publisher` |
| Description | `Allows the fwd-deck release workflow to update Formula and Cask files in oiekjr/homebrew-tap.` |
| Repository access | Only select repositories |
| Selected repositories | `oiekjr/homebrew-tap` |
| Contents | Read and write |
| Metadata | Read-only |
| Other permissions | No access |

`Workflows` permission は不要です。  
release workflow は tapリポジトリの `.github/workflows` を更新せず、`Formula/` と `Casks/` だけを commit します。  

### Set Repository Secret

作成した token は、本体リポジトリ `oiekjr/fwd-deck` の GitHub Actions secret に設定します。  
tapリポジトリ側ではなく、本体リポジトリ側に設定します。  

```text
Repository: oiekjr/fwd-deck
Path: Settings > Secrets and variables > Actions > New repository secret
Secret name: HOMEBREW_TAP_TOKEN
Secret value: 作成した fine-grained personal access token
```

### Protect Release Tags

release workflow は `v*.*.*` の tag push を契機に、GitHub Release 作成、DMG upload、Homebrew tap更新まで実行します。  
誤操作で tag を削除、再作成、上書きすると配布物に影響するため、GitHub の ruleset で release tag を保護します。  

GitHub の `oiekjr/fwd-deck` で次の設定を行います。  

```text
Path: Settings > Rules > Rulesets > New ruleset > New tag ruleset
Ruleset name: release-tags
Enforcement status: Active
Target tags: v*.*.*
Rules:
  - Restrict creations
  - Restrict updates
  - Restrict deletions
  - Require signed commits
Bypass list:
  - Repository admin only
```

`Restrict creations` を有効にすると、release tag を作れる人を bypass許可者に限定できます。  
`Restrict updates` と `Restrict deletions` は、公開済み tag の付け直しや削除を防ぎます。  
`Require signed commits` は、tag が指す commit に署名を要求します。  

GitHub の ruleset は repo管理者、または repository rules を編集できる権限を持つユーザーが設定します。  

### Confirm Tap Access

tap として参照できることを確認します。  

```sh
brew tap oiekjr/tap
brew tap-info oiekjr/tap
```

初回 release が終わるまでは、`Formula/fwd-deck.rb` と `Casks/fwd-deck-app.rb` は存在しない場合があります。  
これらは release workflow が tag push時に作成または更新します。  

## Distribution Policy

利用者向けの導線は次のとおりです。  

```sh
brew install oiekjr/tap/fwd-deck
brew install --cask oiekjr/tap/fwd-deck-app
```

tap済みの環境では、次の短い名前で利用できます。  

```sh
brew install fwd-deck
brew install --cask fwd-deck-app
```

## macOS App Signing

macOSアプリは当面、個人利用向けの unsigned app として配布します。  
Apple Developer Program、Developer ID証明書、notarization credentials は不要です。  

Gatekeeper で起動が止まる場合は、Finder で `/Applications/Fwd Deck.app` を右クリックして Open を選択します。  
Homebrew の quarantine を付けたくない場合は、次のようにインストールします。  

```sh
brew install --cask --no-quarantine oiekjr/tap/fwd-deck-app
```

## Release

リリース前に、Cargo package、Tauri、npm package の version を同じ値に揃えます。  
release workflow は tag から先頭の `v` を除いた値と各 package version が一致することを検証します。  

```sh
task fmt
task app:format
task check
git tag v0.1.0
git push origin v0.1.0
```

tag push後、release workflow は次を実行します。  

1. GitHub Release を作成する
2. unsigned universal macOS DMG を build する
3. DMG と SHA256ファイルを Release asset として添付する
4. GitHub tag archive の SHA256 を算出する
5. `oiekjr/homebrew-tap` の formula と cask を更新して push する

Release asset は上書きしません。  
同じ tag に同名 asset が存在する場合、workflow は失敗します。  
公開済み version を差し替えるのではなく、新しい version を作成してください。  

## Verification

tap更新後に、ローカル環境で Homebrew の検証を行います。  

```sh
brew audit --formula oiekjr/tap/fwd-deck
brew install --build-from-source oiekjr/tap/fwd-deck
brew test oiekjr/tap/fwd-deck
brew audit --cask oiekjr/tap/fwd-deck-app
brew install --cask oiekjr/tap/fwd-deck-app
```

アプリは `/Applications/Fwd Deck.app` として配置されます。  
macOS実機で右クリックで Open を選択するか、`--no-quarantine` installation で起動できることを確認します。  
