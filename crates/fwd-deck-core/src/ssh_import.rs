use std::{fmt, path::Path};

use thiserror::Error;
use uuid::Uuid;

use crate::{DEFAULT_LOCAL_HOST, TimeoutConfig, TunnelConfig};

/// SSHコマンド取り込み時に既定で付与するタグを表現する
pub const DEFAULT_IMPORT_TAG: &str = "imported";

/// SSHコマンド全体の解析結果を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshImport {
    pub forwards: Vec<SshLocalForward>,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<u16>,
    pub identity_file: Option<String>,
    pub timeouts: TimeoutConfig,
    pub warnings: Vec<SshImportWarning>,
}

impl SshImport {
    /// 指定した転送設定からトンネル設定を生成する
    pub fn tunnel_config(
        &self,
        forward: &SshLocalForward,
        name: String,
        description: Option<String>,
        tags: Vec<String>,
    ) -> TunnelConfig {
        TunnelConfig {
            name,
            description,
            tags,
            local_host: Some(forward.local_host.clone()),
            local_port: forward.local_port,
            remote_host: forward.remote_host.clone(),
            remote_port: forward.remote_port,
            ssh_user: self.ssh_user.clone(),
            ssh_host: self.ssh_host.clone(),
            ssh_port: self.ssh_port,
            identity_file: self.identity_file.clone(),
            timeouts: self.timeouts.clone(),
        }
    }
}

/// SSHローカルフォワード1件の解析結果を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshLocalForward {
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

/// SSHコマンド取り込み時に保持できなかった項目を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshImportWarning {
    pub option: String,
    pub message: String,
}

impl SshImportWarning {
    /// 警告を初期化する
    fn new(option: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            option: option.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SshImportWarning {
    /// 警告を表示用文字列へ変換する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.option, self.message)
    }
}

/// SSHコマンド解析時の失敗理由を表現する
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SshImportError {
    #[error("SSHコマンドを入力してください")]
    EmptyCommand,
    #[error("SSHコマンドのshell構文を解析できませんでした")]
    InvalidShellSyntax,
    #[error("SSHローカルフォワード（-L）が指定されていません")]
    MissingLocalForward,
    #[error("SSH接続先ホストが指定されていません")]
    MissingSshDestination,
    #[error("SSHユーザーが指定されていません。user@host または -l USER を指定してください")]
    MissingSshUser,
    #[error("{option} の値が指定されていません")]
    MissingOptionValue { option: String },
    #[error("{field} は1から65535のポート番号で指定してください: {value}")]
    InvalidPort { field: &'static str, value: String },
    #[error("未対応のローカルフォワード形式です: {value}")]
    InvalidLocalForward { value: String },
    #[error("{option} はSSHコマンド取り込みでは未対応です")]
    UnsupportedForwardOption { option: String },
}

/// SSHコマンド文字列を解析する
pub fn parse_ssh_command(command: &str) -> Result<SshImport, SshImportError> {
    if command.trim().is_empty() {
        return Err(SshImportError::EmptyCommand);
    }

    let args = shlex::split(command).ok_or(SshImportError::InvalidShellSyntax)?;

    parse_ssh_args(&args)
}

/// SSHコマンド引数を解析する
pub fn parse_ssh_args(args: &[String]) -> Result<SshImport, SshImportError> {
    let args = ssh_args_without_command_name(args);
    if args.is_empty() {
        return Err(SshImportError::EmptyCommand);
    }

    let mut parser = SshArgParser::default();
    parser.parse(args)?;
    parser.finish()
}

/// 取り込み用の一意性が高いトンネル名を生成する
pub fn generate_import_tunnel_name() -> String {
    let id = Uuid::new_v4().simple().to_string();

    format!("tunnel-{}", &id[..8])
}

#[derive(Debug, Default)]
struct SshArgParser {
    forwards: Vec<SshLocalForward>,
    explicit_user: Option<String>,
    ssh_host: Option<String>,
    ssh_port: Option<u16>,
    identity_file: Option<String>,
    timeouts: TimeoutConfig,
    warnings: Vec<SshImportWarning>,
    destination_seen: bool,
    stop_option_parsing: bool,
}

impl SshArgParser {
    /// 引数列からSSH取り込み情報を抽出する
    fn parse(&mut self, args: &[String]) -> Result<(), SshImportError> {
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];

            if self.stop_option_parsing || !is_option_like(arg) {
                self.handle_positional(arg);
                index += 1;
                continue;
            }

            if arg == "--" {
                self.stop_option_parsing = true;
                index += 1;
                continue;
            }

            if short_option_matches(arg, "-L") {
                let value = take_short_option_value(args, &mut index, "-L")?;
                self.forwards.push(parse_local_forward(&value)?);
                index += 1;
                continue;
            }

            if short_option_matches(arg, "-R") || short_option_matches(arg, "-D") {
                let option = arg.chars().take(2).collect::<String>();
                return Err(SshImportError::UnsupportedForwardOption { option });
            }

            if short_option_matches(arg, "-i") {
                self.identity_file = Some(take_short_option_value(args, &mut index, "-i")?);
                index += 1;
                continue;
            }

            if short_option_matches(arg, "-p") {
                let value = take_short_option_value(args, &mut index, "-p")?;
                self.ssh_port = Some(parse_port("ssh_port", &value)?);
                index += 1;
                continue;
            }

            if short_option_matches(arg, "-l") {
                self.explicit_user = Some(take_short_option_value(args, &mut index, "-l")?);
                index += 1;
                continue;
            }

            if short_option_matches(arg, "-o") {
                let value = take_short_option_value(args, &mut index, "-o")?;
                self.handle_ssh_option(&value)?;
                index += 1;
                continue;
            }

            if ignored_option_without_warning(arg) {
                index += 1;
                continue;
            }

            if let Some(option) = known_value_option(arg) {
                let value = take_short_option_value(args, &mut index, option)?;
                self.warn(
                    option,
                    format!("このSSHオプションは設定ファイルへ保存されません: {value}"),
                );
                index += 1;
                continue;
            }

            self.warn(arg, "このSSHオプションは設定ファイルへ保存されません");
            index += 1;
        }

        Ok(())
    }

    /// 解析結果を必須項目検証済みの取り込み結果へ変換する
    fn finish(self) -> Result<SshImport, SshImportError> {
        if self.forwards.is_empty() {
            return Err(SshImportError::MissingLocalForward);
        }

        let ssh_host = self.ssh_host.ok_or(SshImportError::MissingSshDestination)?;
        let ssh_user = self.explicit_user.ok_or(SshImportError::MissingSshUser)?;

        Ok(SshImport {
            forwards: self.forwards,
            ssh_user,
            ssh_host,
            ssh_port: self.ssh_port,
            identity_file: self.identity_file,
            timeouts: self.timeouts,
            warnings: self.warnings,
        })
    }

    /// SSH接続先やリモートコマンドを処理する
    fn handle_positional(&mut self, value: &str) {
        if !self.destination_seen {
            self.destination_seen = true;
            let (user, host) = split_destination(value);

            if let Some(user) = user {
                self.explicit_user = Some(user);
            }
            self.ssh_host = Some(host);
            return;
        }

        self.warn(value, "リモートコマンド引数は設定ファイルへ保存されません");
    }

    /// -o で渡されたSSHオプションを処理する
    fn handle_ssh_option(&mut self, value: &str) -> Result<(), SshImportError> {
        let Some((key, raw_value)) = split_ssh_option_assignment(value) else {
            self.warn(
                "-o",
                format!("key=value形式ではないSSHオプションは保存されません: {value}"),
            );
            return Ok(());
        };
        let normalized_key = normalize_ssh_option_key(key);

        match normalized_key.as_str() {
            "connecttimeout" => {
                self.timeouts.connect_timeout_seconds =
                    Some(parse_u32_option("ConnectTimeout", raw_value)?);
            }
            "serveraliveinterval" => {
                self.timeouts.server_alive_interval_seconds =
                    Some(parse_u32_option("ServerAliveInterval", raw_value)?);
            }
            "serveralivecountmax" => {
                self.timeouts.server_alive_count_max =
                    Some(parse_u32_option("ServerAliveCountMax", raw_value)?);
            }
            "exitonforwardfailure" => {}
            _ => self.warn(
                format!("-o {key}"),
                "このSSHオプションは設定ファイルへ保存されません",
            ),
        }

        Ok(())
    }

    /// 警告を追加する
    fn warn(&mut self, option: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(SshImportWarning::new(option, message));
    }
}

/// ssh コマンド名が含まれる場合は取り除いた引数列を返す
fn ssh_args_without_command_name(args: &[String]) -> &[String] {
    if args.first().is_some_and(|arg| is_ssh_command_name(arg)) {
        &args[1..]
    } else {
        args
    }
}

/// 引数がSSHコマンド名かを判定する
fn is_ssh_command_name(value: &str) -> bool {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "ssh")
}

/// SSHオプションとして扱える引数かを判定する
fn is_option_like(value: &str) -> bool {
    value.starts_with('-') && value != "-"
}

/// short option が対象オプションに一致するかを判定する
fn short_option_matches(value: &str, option: &str) -> bool {
    value == option
        || value
            .strip_prefix(option)
            .is_some_and(|rest| !rest.is_empty())
}

/// short option の値を取得する
fn take_short_option_value(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<String, SshImportError> {
    let arg = &args[*index];

    if arg == option {
        *index += 1;
        return args
            .get(*index)
            .cloned()
            .ok_or_else(|| SshImportError::MissingOptionValue {
                option: option.to_owned(),
            });
    }

    let value = arg
        .strip_prefix(option)
        .expect("caller should pass a matching short option")
        .strip_prefix('=')
        .unwrap_or_else(|| arg.strip_prefix(option).expect("matching short option"));

    if value.is_empty() {
        return Err(SshImportError::MissingOptionValue {
            option: option.to_owned(),
        });
    }

    Ok(value.to_owned())
}

/// 警告なしで無視できるSSHオプションかを判定する
fn ignored_option_without_warning(value: &str) -> bool {
    value == "-N" || value.chars().skip(1).all(|character| character == 'v')
}

/// 値を持つ既知のSSHオプションならオプション名を返す
fn known_value_option(value: &str) -> Option<&'static str> {
    const OPTIONS: [&str; 14] = [
        "-B", "-b", "-c", "-E", "-e", "-F", "-I", "-J", "-m", "-O", "-Q", "-S", "-W", "-w",
    ];

    OPTIONS
        .into_iter()
        .find(|option| short_option_matches(value, option))
}

/// SSH接続先指定をユーザーとホストへ分解する
fn split_destination(value: &str) -> (Option<String>, String) {
    if let Some((user, host)) = value.rsplit_once('@') {
        (Some(user.to_owned()), strip_brackets(host).to_owned())
    } else {
        (None, strip_brackets(value).to_owned())
    }
}

/// -o の key=value 指定を分割する
fn split_ssh_option_assignment(value: &str) -> Option<(&str, &str)> {
    value
        .split_once('=')
        .filter(|(key, raw_value)| !key.trim().is_empty() && !raw_value.trim().is_empty())
}

/// SSHオプション名を比較用へ正規化する
fn normalize_ssh_option_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '-' && *character != '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// u32 のSSHオプション値を解析する
fn parse_u32_option(field: &'static str, value: &str) -> Result<u32, SshImportError> {
    value
        .parse::<u32>()
        .map_err(|_| SshImportError::InvalidPort {
            field,
            value: value.to_owned(),
        })
}

/// ローカルフォワード指定を解析する
fn parse_local_forward(value: &str) -> Result<SshLocalForward, SshImportError> {
    if value.starts_with('/') {
        return Err(SshImportError::InvalidLocalForward {
            value: value.to_owned(),
        });
    }

    let parts = split_colon_outside_brackets(value);
    let (local_host, local_port, remote_host, remote_port) = match parts.as_slice() {
        [local_port, remote_host, remote_port] => (
            DEFAULT_LOCAL_HOST.to_owned(),
            parse_port("local_port", local_port)?,
            strip_brackets(remote_host).to_owned(),
            parse_port("remote_port", remote_port)?,
        ),
        [local_host, local_port, remote_host, remote_port] if !local_host.is_empty() => (
            strip_brackets(local_host).to_owned(),
            parse_port("local_port", local_port)?,
            strip_brackets(remote_host).to_owned(),
            parse_port("remote_port", remote_port)?,
        ),
        _ => {
            return Err(SshImportError::InvalidLocalForward {
                value: value.to_owned(),
            });
        }
    };

    if remote_host.is_empty() {
        return Err(SshImportError::InvalidLocalForward {
            value: value.to_owned(),
        });
    }

    Ok(SshLocalForward {
        local_host,
        local_port,
        remote_host,
        remote_port,
    })
}

/// ポート文字列を検証済みの数値へ変換する
fn parse_port(field: &'static str, value: &str) -> Result<u16, SshImportError> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| SshImportError::InvalidPort {
            field,
            value: value.to_owned(),
        })
}

/// 角括弧内のIPv6区切りを保護しながらコロン区切りに分割する
fn split_colon_outside_brackets(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut bracket_depth = 0u8;

    for character in value.chars() {
        match character {
            '[' => {
                bracket_depth = bracket_depth.saturating_add(1);
                current.push(character);
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                current.push(character);
            }
            ':' if bracket_depth == 0 => {
                parts.push(current);
                current = String::new();
            }
            _ => current.push(character),
        }
    }

    parts.push(current);
    parts
}

/// IPv6などで使う外側の角括弧を取り除く
fn strip_brackets(value: &str) -> &str {
    value
        .strip_prefix('[')
        .and_then(|without_prefix| without_prefix.strip_suffix(']'))
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 提示形式のSSHコマンドから複数トンネル候補を解析する
    #[test]
    fn parse_sample_ssh_command_extracts_multiple_forwards() {
        let import = parse_ssh_command(
            "ssh -v -N -L localhost:45432:imc-prod.example.com:5432 -L localhost:45433:imc-dev.example.com:5432 -o ConnectTimeout=15 -o ServerAliveInterval=30 -o ServerAliveCountMax=3 -o TCPKeepAlive=yes -i /Users/example/.ssh/key.pem -p 22 ec2-user@54.199.2.184",
        )
        .expect("parse ssh command");

        assert_eq!(import.forwards.len(), 2);
        assert_eq!(import.forwards[0].local_host, "localhost");
        assert_eq!(import.forwards[0].local_port, 45432);
        assert_eq!(import.forwards[0].remote_host, "imc-prod.example.com");
        assert_eq!(import.forwards[0].remote_port, 5432);
        assert_eq!(import.forwards[1].local_port, 45433);
        assert_eq!(import.ssh_user, "ec2-user");
        assert_eq!(import.ssh_host, "54.199.2.184");
        assert_eq!(import.ssh_port, Some(22));
        assert_eq!(
            import.identity_file.as_deref(),
            Some("/Users/example/.ssh/key.pem")
        );
        assert_eq!(import.timeouts.connect_timeout_seconds, Some(15));
        assert_eq!(import.timeouts.server_alive_interval_seconds, Some(30));
        assert_eq!(import.timeouts.server_alive_count_max, Some(3));
        assert!(import.warnings.iter().any(|warning| {
            warning.option == "-o TCPKeepAlive"
                && warning.message.contains("設定ファイルへ保存されません")
        }));
    }

    /// bind address を省略したローカルフォワードを既定ホストで解析する
    #[test]
    fn parse_local_forward_without_bind_address_uses_default_host() {
        let import = parse_ssh_command("ssh -L 45432:db.example.com:5432 -l ec2-user bastion")
            .expect("parse ssh command");

        assert_eq!(import.forwards[0].local_host, DEFAULT_LOCAL_HOST);
        assert_eq!(import.forwards[0].local_port, 45432);
        assert_eq!(import.forwards[0].remote_host, "db.example.com");
        assert_eq!(import.forwards[0].remote_port, 5432);
        assert_eq!(import.ssh_user, "ec2-user");
        assert_eq!(import.ssh_host, "bastion");
    }

    /// ssh コマンド名なしの引数列を解析する
    #[test]
    fn parse_ssh_args_accepts_args_without_command_name() {
        let args = ["-L", "15432:db.example.com:5432", "user@bastion"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        let import = parse_ssh_args(&args).expect("parse ssh args");

        assert_eq!(import.forwards[0].local_port, 15432);
        assert_eq!(import.ssh_user, "user");
        assert_eq!(import.ssh_host, "bastion");
    }

    /// 壊れたローカルフォワード指定をエラーとして扱う
    #[test]
    fn parse_ssh_command_rejects_invalid_local_forward() {
        let result =
            parse_ssh_command("ssh -L localhost:notaport:db.example.com:5432 user@bastion");

        assert_eq!(
            result,
            Err(SshImportError::InvalidPort {
                field: "local_port",
                value: "notaport".to_owned()
            })
        );
    }

    /// SSHユーザーが取得できないコマンドをエラーとして扱う
    #[test]
    fn parse_ssh_command_rejects_missing_ssh_user() {
        let result = parse_ssh_command("ssh -L 15432:db.example.com:5432 bastion");

        assert_eq!(result, Err(SshImportError::MissingSshUser));
    }

    /// ローカルフォワードがないコマンドをエラーとして扱う
    #[test]
    fn parse_ssh_command_rejects_missing_forward() {
        let result = parse_ssh_command("ssh -N user@bastion");

        assert_eq!(result, Err(SshImportError::MissingLocalForward));
    }

    /// リモートフォワードや動的フォワードを未対応として拒否する
    #[test]
    fn parse_ssh_command_rejects_unsupported_forward_options() {
        let result = parse_ssh_command("ssh -R 15432:db.example.com:5432 user@bastion");

        assert_eq!(
            result,
            Err(SshImportError::UnsupportedForwardOption {
                option: "-R".to_owned()
            })
        );
    }
}
