# Security Policy

## Supported Versions

`fwd-deck` は、最新の公開リリースのみをセキュリティ対応の対象とします。

古いバージョンで確認された問題は、まず最新リリースで再現するかを確認してください。

## Reporting a Vulnerability

脆弱性を見つけた場合は、公開Issueには書き込まず、GitHub の [Private Vulnerability Reporting](https://github.com/oiekjr/fwd-deck/security/advisories/new) から報告してください。

報告には、可能な範囲で次の情報を含めてください。

- 影響を受ける `fwd-deck` のバージョン
- 利用環境のOSとインストール方法
- 再現手順
- 想定される影響
- 回避策がある場合はその内容

設定ファイル、SSH接続先、秘密鍵、token、credential などの機密情報は含めないでください。

報告内容を確認し、再現性と影響範囲を判断したうえで対応方針を検討します。

個人開発プロジェクトのため、返信時期と修正時期は保証しません。

## Scope

`fwd-deck` 本体、CLI、macOSアプリ、release workflow、配布用artifactに関する脆弱性を対象とします。

利用者自身のSSHサーバー設定、ローカルネットワーク設定、公開済みの個人設定ファイルに起因する問題は、原則として対象外です。
