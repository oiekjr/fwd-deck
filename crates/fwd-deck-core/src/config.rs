use std::{
    collections::{HashMap, hash_map::Entry},
    env,
    fmt::{self, Display},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const GLOBAL_CONFIG_RELATIVE_PATH: &str = ".config/fwd-deck/config.toml";
const LOCAL_CONFIG_FILE_NAME: &str = "fwd-deck.toml";
const DEFAULT_LOCAL_HOST: &str = "127.0.0.1";

/// 読み込む設定ファイルの位置を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub global: Option<PathBuf>,
    pub local: PathBuf,
}

impl ConfigPaths {
    /// 設定ファイルの位置を初期化する
    pub fn new(global: Option<PathBuf>, local: PathBuf) -> Self {
        Self { global, local }
    }
}

/// 設定ファイルの種類を表現する
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigSourceKind {
    Global,
    Local,
}

impl Display for ConfigSourceKind {
    /// 設定ファイルの種類を表示用文字列へ変換する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Global => formatter.write_str("global"),
            Self::Local => formatter.write_str("local"),
        }
    }
}

/// 設定ファイルの由来を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSource {
    pub kind: ConfigSourceKind,
    pub path: PathBuf,
}

impl ConfigSource {
    /// 設定ファイルの由来を初期化する
    pub fn new(kind: ConfigSourceKind, path: PathBuf) -> Self {
        Self { kind, path }
    }
}

/// TOML に記述するトンネル設定を表現する
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TunnelConfig {
    pub id: String,
    pub description: Option<String>,
    pub local_host: Option<String>,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<u16>,
    pub identity_file: Option<String>,
}

impl TunnelConfig {
    /// 未指定時の既定値を含めたローカルホストを取得する
    pub fn effective_local_host(&self) -> &str {
        self.local_host.as_deref().unwrap_or(DEFAULT_LOCAL_HOST)
    }
}

/// 読み込み済み設定ファイルの内容を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfigFile {
    pub source: ConfigSource,
    pub tunnels: Vec<TunnelConfig>,
}

impl LoadedConfigFile {
    /// 読み込み済み設定ファイルを初期化する
    pub fn new(source: ConfigSource, tunnels: Vec<TunnelConfig>) -> Self {
        Self { source, tunnels }
    }

    /// 設定ファイル内のトンネル設定を統合用の形式へ変換する
    fn resolved_tunnels(&self) -> impl Iterator<Item = ResolvedTunnelConfig> + '_ {
        self.tunnels
            .iter()
            .cloned()
            .map(|tunnel| ResolvedTunnelConfig::new(self.source.clone(), tunnel))
    }
}

/// 統合後のトンネル設定を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTunnelConfig {
    pub source: ConfigSource,
    pub tunnel: TunnelConfig,
}

impl ResolvedTunnelConfig {
    /// 統合後のトンネル設定を初期化する
    pub fn new(source: ConfigSource, tunnel: TunnelConfig) -> Self {
        Self { source, tunnel }
    }
}

/// 統合済み設定全体を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub sources: Vec<LoadedConfigFile>,
    pub tunnels: Vec<ResolvedTunnelConfig>,
}

impl EffectiveConfig {
    /// 統合済み設定を初期化する
    pub fn new(sources: Vec<LoadedConfigFile>, tunnels: Vec<ResolvedTunnelConfig>) -> Self {
        Self { sources, tunnels }
    }

    /// 設定ファイルが読み込まれているかを判定する
    pub fn has_sources(&self) -> bool {
        !self.sources.is_empty()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfigFile {
    #[serde(default)]
    tunnels: Vec<TunnelConfig>,
}

/// 設定読込時の失敗理由を表現する
#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("Failed to read configuration file: {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse TOML configuration file: {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

/// 設定検証の結果を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    /// 検証エラーを含まない結果を初期化する
    pub fn valid() -> Self {
        Self { errors: Vec::new() }
    }

    /// 検証結果が成功かを判定する
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// 検証エラーを追加する
    pub fn push(&mut self, error: ValidationError) {
        self.errors.push(error);
    }
}

/// 設定検証で検出した問題を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub source: ConfigSource,
    pub tunnel_id: Option<String>,
    pub message: String,
}

impl ValidationError {
    /// 検証エラーを初期化する
    pub fn new(
        source: ConfigSource,
        tunnel_id: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source,
            tunnel_id,
            message: message.into(),
        }
    }
}

/// グローバル設定ファイルの既定パスを取得する
pub fn default_global_config_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(GLOBAL_CONFIG_RELATIVE_PATH))
}

/// ローカル設定ファイルの既定パスを取得する
pub fn default_local_config_path(current_dir: &Path) -> PathBuf {
    current_dir.join(LOCAL_CONFIG_FILE_NAME)
}

/// 指定された設定ファイルを読み込む
pub fn read_config_file(
    path: &Path,
    kind: ConfigSourceKind,
) -> Result<Option<LoadedConfigFile>, ConfigLoadError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|source| ConfigLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw =
        toml::from_str::<RawConfigFile>(&content).map_err(|source| ConfigLoadError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(Some(LoadedConfigFile::new(
        ConfigSource::new(kind, path.to_path_buf()),
        raw.tunnels,
    )))
}

/// グローバル設定とローカル設定を統合して読み込む
pub fn load_effective_config(paths: &ConfigPaths) -> Result<EffectiveConfig, ConfigLoadError> {
    let sources = read_existing_config_files(paths)?;
    let tunnels = merge_tunnels(&sources);

    Ok(EffectiveConfig::new(sources, tunnels))
}

/// 設定内容の意味的な不備を検証する
pub fn validate_config(config: &EffectiveConfig) -> ValidationReport {
    let mut report = ValidationReport::valid();

    validate_duplicate_ids(config, &mut report);
    validate_required_fields(config, &mut report);
    validate_optional_fields(config, &mut report);
    validate_ports(config, &mut report);
    validate_duplicate_local_ports(config, &mut report);

    report
}

/// 任意項目が指定された場合の制約を検証する
fn validate_optional_fields(config: &EffectiveConfig, report: &mut ValidationReport) {
    for resolved in &config.tunnels {
        if let Some(local_host) = &resolved.tunnel.local_host {
            validate_local_host(resolved, local_host, report);
        }
    }
}

/// ローカル側の bind address として扱える値かを検証する
fn validate_local_host(
    resolved: &ResolvedTunnelConfig,
    local_host: &str,
    report: &mut ValidationReport,
) {
    if local_host.trim().is_empty() {
        report.push(ValidationError::new(
            resolved.source.clone(),
            Some(resolved.tunnel.id.clone()),
            "local_host cannot be empty",
        ));
        return;
    }

    if local_host.chars().any(char::is_whitespace) {
        report.push(ValidationError::new(
            resolved.source.clone(),
            Some(resolved.tunnel.id.clone()),
            "local_host cannot contain whitespace",
        ));
    }
}

/// 設定ファイルの優先順位に従ってトンネル設定を統合する
fn merge_tunnels(sources: &[LoadedConfigFile]) -> Vec<ResolvedTunnelConfig> {
    let mut tunnels = Vec::new();
    let mut positions = HashMap::<String, usize>::new();

    for file in sources {
        for resolved in file.resolved_tunnels() {
            match positions.entry(resolved.tunnel.id.clone()) {
                Entry::Occupied(entry) => {
                    tunnels[*entry.get()] = resolved;
                }
                Entry::Vacant(entry) => {
                    entry.insert(tunnels.len());
                    tunnels.push(resolved);
                }
            }
        }
    }

    tunnels
}

/// 存在する設定ファイルを既定の優先順位で読み込む
fn read_existing_config_files(
    paths: &ConfigPaths,
) -> Result<Vec<LoadedConfigFile>, ConfigLoadError> {
    let mut sources = Vec::new();

    if let Some(global_path) = &paths.global
        && let Some(file) = read_config_file(global_path, ConfigSourceKind::Global)?
    {
        sources.push(file);
    }

    if let Some(file) = read_config_file(&paths.local, ConfigSourceKind::Local)? {
        sources.push(file);
    }

    Ok(sources)
}

/// 同一設定ファイル内の ID 重複を検証する
fn validate_duplicate_ids(config: &EffectiveConfig, report: &mut ValidationReport) {
    for file in &config.sources {
        let mut counts = HashMap::<&str, usize>::new();

        for tunnel in &file.tunnels {
            *counts.entry(tunnel.id.as_str()).or_default() += 1;
        }

        for (id, count) in counts {
            if count > 1 {
                report.push(ValidationError::new(
                    file.source.clone(),
                    Some(id.to_owned()),
                    "Duplicate id in the same configuration file",
                ));
            }
        }
    }
}

/// 必須項目が空文字列ではないことを検証する
fn validate_required_fields(config: &EffectiveConfig, report: &mut ValidationReport) {
    for resolved in &config.tunnels {
        for (field_name, value) in required_string_fields(&resolved.tunnel) {
            validate_non_empty(
                &resolved.source,
                &resolved.tunnel,
                field_name,
                value,
                report,
            );
        }
    }
}

/// 空文字列を禁止する項目を取得する
fn required_string_fields(tunnel: &TunnelConfig) -> [(&'static str, &str); 4] {
    [
        ("id", tunnel.id.as_str()),
        ("remote_host", tunnel.remote_host.as_str()),
        ("ssh_user", tunnel.ssh_user.as_str()),
        ("ssh_host", tunnel.ssh_host.as_str()),
    ]
}

/// 空文字列の項目を検証結果へ追加する
fn validate_non_empty(
    source: &ConfigSource,
    tunnel: &TunnelConfig,
    field_name: &str,
    value: &str,
    report: &mut ValidationReport,
) {
    if value.trim().is_empty() {
        report.push(ValidationError::new(
            source.clone(),
            Some(tunnel.id.clone()),
            format!("{field_name} cannot be empty"),
        ));
    }
}

/// ポート番号が有効範囲であることを検証する
fn validate_ports(config: &EffectiveConfig, report: &mut ValidationReport) {
    for resolved in &config.tunnels {
        for (field_name, port) in port_fields(&resolved.tunnel) {
            validate_non_zero_port(resolved, field_name, port, report);
        }
    }
}

/// 検証対象のポート項目を取得する
fn port_fields(tunnel: &TunnelConfig) -> [(&'static str, u16); 2] {
    [
        ("local_port", tunnel.local_port),
        ("remote_port", tunnel.remote_port),
    ]
}

/// ポート番号が 0 ではないことを検証する
fn validate_non_zero_port(
    resolved: &ResolvedTunnelConfig,
    field_name: &str,
    port: u16,
    report: &mut ValidationReport,
) {
    if port == 0 {
        report.push(ValidationError::new(
            resolved.source.clone(),
            Some(resolved.tunnel.id.clone()),
            format!("{field_name} must be greater than or equal to 1"),
        ));
    }
}

/// 統合後設定のローカルポート重複を検証する
fn validate_duplicate_local_ports(config: &EffectiveConfig, report: &mut ValidationReport) {
    let mut ports = HashMap::<u16, &ResolvedTunnelConfig>::new();

    for resolved in &config.tunnels {
        if let Some(existing) = ports.insert(resolved.tunnel.local_port, resolved) {
            report.push(ValidationError::new(
                resolved.source.clone(),
                Some(resolved.tunnel.id.clone()),
                format!(
                    "local_port {} duplicates {}",
                    resolved.tunnel.local_port, existing.tunnel.id
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// ローカル設定が同一 ID のグローバル設定を上書きすることを検証する
    #[test]
    fn local_config_overrides_global_config_by_id() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let global_path = temp_dir.path().join("global.toml");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &global_path,
            r#"
[[tunnels]]
id = "db"
local_port = 15432
remote_host = "global-db.internal"
remote_port = 5432
ssh_user = "global-user"
ssh_host = "global-bastion.example.com"
"#,
        )
        .expect("write global configuration");
        fs::write(
            &local_path,
            r#"
[[tunnels]]
id = "db"
local_port = 25432
remote_host = "local-db.internal"
remote_port = 5432
ssh_user = "local-user"
ssh_host = "local-bastion.example.com"
"#,
        )
        .expect("write local configuration");

        let config = load_effective_config(&ConfigPaths::new(Some(global_path), local_path))
            .expect("load configuration");

        assert_eq!(config.tunnels.len(), 1);
        assert_eq!(config.tunnels[0].source.kind, ConfigSourceKind::Local);
        assert_eq!(config.tunnels[0].tunnel.local_port, 25432);
        assert_eq!(config.tunnels[0].tunnel.remote_host, "local-db.internal");
    }

    /// local_host 未指定時に既定値が使われることを検証する
    #[test]
    fn tunnel_config_uses_default_local_host_when_omitted() {
        let tunnel = tunnel("db", 15432);

        assert_eq!(tunnel.effective_local_host(), "127.0.0.1");
    }

    /// 同一設定ファイル内の ID 重複が検証エラーになることを検証する
    #[test]
    fn validation_reports_duplicate_ids_in_same_file() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let config = EffectiveConfig::new(
            vec![LoadedConfigFile::new(
                source.clone(),
                vec![tunnel("db", 15432), tunnel("db", 25432)],
            )],
            vec![ResolvedTunnelConfig::new(
                source.clone(),
                tunnel("db", 25432),
            )],
        );

        let report = validate_config(&config);

        assert!(!report.is_valid());
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message == "Duplicate id in the same configuration file")
        );
    }

    /// 統合後設定のローカルポート重複が検証エラーになることを検証する
    #[test]
    fn validation_reports_duplicate_local_ports() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let config = EffectiveConfig::new(
            vec![LoadedConfigFile::new(
                source.clone(),
                vec![tunnel("db", 15432), tunnel("cache", 15432)],
            )],
            vec![
                ResolvedTunnelConfig::new(source.clone(), tunnel("db", 15432)),
                ResolvedTunnelConfig::new(source, tunnel("cache", 15432)),
            ],
        );

        let report = validate_config(&config);

        assert!(!report.is_valid());
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message == "local_port 15432 duplicates db")
        );
    }

    /// 空白文字を含む local_host が検証エラーになることを検証する
    #[test]
    fn validation_reports_local_host_with_whitespace() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let mut invalid_tunnel = tunnel("db", 15432);
        invalid_tunnel.local_host = Some("127.0.0.1 ".to_owned());
        let config = EffectiveConfig::new(
            vec![LoadedConfigFile::new(
                source.clone(),
                vec![invalid_tunnel.clone()],
            )],
            vec![ResolvedTunnelConfig::new(source, invalid_tunnel)],
        );

        let report = validate_config(&config);

        assert!(!report.is_valid());
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message == "local_host cannot contain whitespace")
        );
    }

    /// テスト用のトンネル設定を生成する
    fn tunnel(id: &str, local_port: u16) -> TunnelConfig {
        TunnelConfig {
            id: id.to_owned(),
            description: None,
            local_host: None,
            local_port,
            remote_host: "db.internal".to_owned(),
            remote_port: 5432,
            ssh_user: "user".to_owned(),
            ssh_host: "bastion.example.com".to_owned(),
            ssh_port: None,
            identity_file: None,
        }
    }
}
