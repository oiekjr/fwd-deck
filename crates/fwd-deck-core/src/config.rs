use std::{
    collections::{HashMap, hash_map::Entry},
    env,
    fmt::{self, Display},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::path_display::format_path_for_display;

const GLOBAL_CONFIG_RELATIVE_PATH: &str = ".config/fwd-deck/config.toml";
const LOCAL_CONFIG_FILE_NAME: &str = "fwd-deck.toml";
pub const DEFAULT_LOCAL_HOST: &str = "127.0.0.1";
pub const DEFAULT_CONNECT_TIMEOUT_SECONDS: u32 = 15;
pub const DEFAULT_SERVER_ALIVE_INTERVAL_SECONDS: u32 = 30;
pub const DEFAULT_SERVER_ALIVE_COUNT_MAX: u32 = 3;
pub const DEFAULT_START_GRACE_MILLISECONDS: u64 = 300;

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

/// TOML に記述するタイムアウト設定を表現する
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimeoutConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_alive_interval_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_alive_count_max: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_grace_milliseconds: Option<u64>,
}

impl TimeoutConfig {
    /// TOML 出力を省略できる空設定かを判定する
    pub fn is_empty(&self) -> bool {
        self.connect_timeout_seconds.is_none()
            && self.server_alive_interval_seconds.is_none()
            && self.server_alive_count_max.is_none()
            && self.start_grace_milliseconds.is_none()
    }

    /// 優先度の高い設定で上書きしたタイムアウト設定を生成する
    fn apply_overrides(&self, overrides: &Self) -> Self {
        Self {
            connect_timeout_seconds: overrides
                .connect_timeout_seconds
                .or(self.connect_timeout_seconds),
            server_alive_interval_seconds: overrides
                .server_alive_interval_seconds
                .or(self.server_alive_interval_seconds),
            server_alive_count_max: overrides
                .server_alive_count_max
                .or(self.server_alive_count_max),
            start_grace_milliseconds: overrides
                .start_grace_milliseconds
                .or(self.start_grace_milliseconds),
        }
    }

    /// 既定値を補って実行時タイムアウト設定を生成する
    fn resolve_with_defaults(&self) -> ResolvedTimeoutConfig {
        self.resolve_with_base(ResolvedTimeoutConfig::default())
    }

    /// 基準値を補って実行時タイムアウト設定を生成する
    fn resolve_with_base(&self, base: ResolvedTimeoutConfig) -> ResolvedTimeoutConfig {
        ResolvedTimeoutConfig {
            connect_timeout_seconds: self
                .connect_timeout_seconds
                .unwrap_or(base.connect_timeout_seconds),
            server_alive_interval_seconds: self
                .server_alive_interval_seconds
                .unwrap_or(base.server_alive_interval_seconds),
            server_alive_count_max: self
                .server_alive_count_max
                .unwrap_or(base.server_alive_count_max),
            start_grace_milliseconds: self
                .start_grace_milliseconds
                .unwrap_or(base.start_grace_milliseconds),
        }
    }
}

/// 実行時に使用する解決済みタイムアウト設定を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTimeoutConfig {
    pub connect_timeout_seconds: u32,
    pub server_alive_interval_seconds: u32,
    pub server_alive_count_max: u32,
    pub start_grace_milliseconds: u64,
}

impl Default for ResolvedTimeoutConfig {
    /// 未指定時のタイムアウト設定を初期化する
    fn default() -> Self {
        Self {
            connect_timeout_seconds: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            server_alive_interval_seconds: DEFAULT_SERVER_ALIVE_INTERVAL_SECONDS,
            server_alive_count_max: DEFAULT_SERVER_ALIVE_COUNT_MAX,
            start_grace_milliseconds: DEFAULT_START_GRACE_MILLISECONDS,
        }
    }
}

/// TOML に記述するトンネル設定を表現する
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TunnelConfig {
    pub id: String,
    pub description: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_tags",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tags: Vec<String>,
    pub local_host: Option<String>,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<u16>,
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "TimeoutConfig::is_empty")]
    pub timeouts: TimeoutConfig,
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
    pub timeouts: TimeoutConfig,
    pub tunnels: Vec<TunnelConfig>,
}

impl LoadedConfigFile {
    /// 読み込み済み設定ファイルを初期化する
    pub fn new(source: ConfigSource, tunnels: Vec<TunnelConfig>) -> Self {
        Self::with_timeouts(source, TimeoutConfig::default(), tunnels)
    }

    /// タイムアウト設定を含む読み込み済み設定ファイルを初期化する
    pub fn with_timeouts(
        source: ConfigSource,
        timeouts: TimeoutConfig,
        tunnels: Vec<TunnelConfig>,
    ) -> Self {
        Self {
            source,
            timeouts,
            tunnels,
        }
    }

    /// 設定ファイル内のトンネル設定を統合用の形式へ変換する
    fn resolved_tunnels(
        &self,
        base_timeouts: ResolvedTimeoutConfig,
    ) -> impl Iterator<Item = ResolvedTunnelConfig> + '_ {
        self.tunnels.iter().cloned().map(move |tunnel| {
            let timeouts = tunnel.timeouts.resolve_with_base(base_timeouts);
            ResolvedTunnelConfig::new_with_timeouts(self.source.clone(), tunnel, timeouts)
        })
    }
}

/// 統合後のトンネル設定を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTunnelConfig {
    pub source: ConfigSource,
    pub tunnel: TunnelConfig,
    pub timeouts: ResolvedTimeoutConfig,
}

impl ResolvedTunnelConfig {
    /// 統合後のトンネル設定を初期化する
    pub fn new(source: ConfigSource, tunnel: TunnelConfig) -> Self {
        Self::new_with_timeouts(source, tunnel, ResolvedTimeoutConfig::default())
    }

    /// 解決済みタイムアウト設定を含む統合後のトンネル設定を初期化する
    pub fn new_with_timeouts(
        source: ConfigSource,
        tunnel: TunnelConfig,
        timeouts: ResolvedTimeoutConfig,
    ) -> Self {
        Self {
            source,
            tunnel,
            timeouts,
        }
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawConfigFile {
    #[serde(default, skip_serializing_if = "TimeoutConfig::is_empty")]
    timeouts: TimeoutConfig,
    #[serde(default)]
    tunnels: Vec<TunnelConfig>,
}

/// 設定読込時の失敗理由を表現する
#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error(
        "Failed to read configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Failed to parse TOML configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

/// 設定編集時の失敗理由を表現する
#[derive(Debug, Error)]
pub enum ConfigEditError {
    #[error(
        "Configuration file was not found: {}",
        format_path_for_display(.path)
    )]
    Missing { path: PathBuf },
    #[error(
        "Tunnel id already exists in configuration file: {id} ({})",
        format_path_for_display(.path)
    )]
    DuplicateId { path: PathBuf, id: String },
    #[error(
        "Tunnel id was not found in configuration file: {id} ({})",
        format_path_for_display(.path)
    )]
    NotFound { path: PathBuf, id: String },
    #[error(
        "Failed to read configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Failed to parse TOML configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error(
        "Failed to serialize TOML configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error(
        "Failed to create configuration directory: {}: {source}",
        format_path_for_display(.path)
    )]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Failed to write configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// 設定検証の結果を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationReport {
    /// 検証エラーを含まない結果を初期化する
    pub fn valid() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// 検証結果がエラーを含まないかを判定する
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// 検証結果が警告を含むかを判定する
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// 検証エラーを追加する
    pub fn push(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    /// 検証警告を追加する
    pub fn push_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
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

/// 設定検証で検出した注意事項を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationWarning {
    pub source: ConfigSource,
    pub tunnel_id: Option<String>,
    pub message: String,
}

impl ValidationWarning {
    /// 検証警告を初期化する
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

    Ok(Some(LoadedConfigFile::with_timeouts(
        ConfigSource::new(kind, path.to_path_buf()),
        raw.timeouts,
        normalize_tunnels(raw.tunnels),
    )))
}

/// 指定された設定ファイルへトンネル設定を追加する
pub fn add_tunnel_to_config_file(
    path: &Path,
    kind: ConfigSourceKind,
    tunnel: TunnelConfig,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let mut file = read_config_file_for_edit(path, kind, true)?;
    let tunnel = normalize_tunnel(tunnel);

    if file.tunnels.iter().any(|existing| existing.id == tunnel.id) {
        return Err(ConfigEditError::DuplicateId {
            path: path.to_path_buf(),
            id: tunnel.id,
        });
    }

    file.tunnels.push(tunnel);
    write_config_file(path, &file)?;

    Ok(file)
}

/// 指定された設定ファイルからトンネル設定を削除する
pub fn remove_tunnel_from_config_file(
    path: &Path,
    kind: ConfigSourceKind,
    id: &str,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let mut file = read_config_file_for_edit(path, kind, false)?;
    let Some(position) = file.tunnels.iter().position(|tunnel| tunnel.id == id) else {
        return Err(ConfigEditError::NotFound {
            path: path.to_path_buf(),
            id: id.to_owned(),
        });
    };

    file.tunnels.remove(position);
    write_config_file(path, &file)?;

    Ok(file)
}

/// 指定された設定ファイル内のトンネル設定を更新する
pub fn update_tunnel_in_config_file(
    path: &Path,
    kind: ConfigSourceKind,
    id: &str,
    tunnel: TunnelConfig,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let mut file = read_config_file_for_edit(path, kind, false)?;
    let tunnel = normalize_tunnel(tunnel);
    let Some(position) = file.tunnels.iter().position(|existing| existing.id == id) else {
        return Err(ConfigEditError::NotFound {
            path: path.to_path_buf(),
            id: id.to_owned(),
        });
    };

    if file
        .tunnels
        .iter()
        .enumerate()
        .any(|(index, existing)| index != position && existing.id == tunnel.id)
    {
        return Err(ConfigEditError::DuplicateId {
            path: path.to_path_buf(),
            id: tunnel.id,
        });
    }

    file.tunnels[position] = tunnel;
    write_config_file(path, &file)?;

    Ok(file)
}

/// グローバル設定とローカル設定を統合して読み込む
pub fn load_effective_config(paths: &ConfigPaths) -> Result<EffectiveConfig, ConfigLoadError> {
    let sources = read_existing_config_files(paths)?;
    let tunnels = merge_tunnels(&sources);

    Ok(EffectiveConfig::new(sources, tunnels))
}

/// タグを比較用の表記へ正規化する
pub fn normalize_tag(tag: &str) -> String {
    tag.trim().to_ascii_lowercase()
}

/// タグ一覧を比較用の表記へ正規化する
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    tags.iter().map(|tag| normalize_tag(tag)).collect()
}

/// タグが許可された ASCII slug かを判定する
pub fn tag_is_valid(tag: &str) -> bool {
    !tag.is_empty()
        && tag
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '/'))
}

/// トンネルが指定されたタグをすべて持つかを判定する
pub fn tunnel_matches_tags(tunnel: &TunnelConfig, required_tags: &[String]) -> bool {
    let required_tags = normalize_tags(required_tags);

    tunnel_matches_normalized_tags(tunnel, &required_tags)
}

/// 指定タグをすべて持つ統合済みトンネル設定を取得する
pub fn filter_tunnels_by_tags<'a>(
    tunnels: &'a [ResolvedTunnelConfig],
    required_tags: &[String],
) -> Vec<&'a ResolvedTunnelConfig> {
    let required_tags = normalize_tags(required_tags);

    tunnels
        .iter()
        .filter(|resolved| tunnel_matches_normalized_tags(&resolved.tunnel, &required_tags))
        .collect()
}

/// 正規化済みタグ条件でトンネルを照合する
fn tunnel_matches_normalized_tags(tunnel: &TunnelConfig, required_tags: &[String]) -> bool {
    if required_tags.is_empty() {
        return true;
    }

    required_tags.iter().all(|required| {
        tunnel
            .tags
            .iter()
            .any(|tag| tag_matches_normalized(tag, required))
    })
}

/// 正規化済み条件とトンネル側タグを比較する
fn tag_matches_normalized(tag: &str, normalized_required: &str) -> bool {
    tag.trim().eq_ignore_ascii_case(normalized_required)
}

/// 設定内容の意味的な不備を検証する
pub fn validate_config(config: &EffectiveConfig) -> ValidationReport {
    let mut report = ValidationReport::valid();

    validate_duplicate_ids(config, &mut report);
    validate_required_fields(config, &mut report);
    validate_optional_fields(config, &mut report);
    validate_tags(config, &mut report);
    validate_ports(config, &mut report);
    warn_privileged_local_ports(config, &mut report);
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

/// タグが許可された形式で記述されているかを検証する
fn validate_tags(config: &EffectiveConfig, report: &mut ValidationReport) {
    for resolved in &config.tunnels {
        for tag in &resolved.tunnel.tags {
            if !tag_is_valid(tag) {
                report.push(ValidationError::new(
                    resolved.source.clone(),
                    Some(resolved.tunnel.id.clone()),
                    format!(
                        "tag must contain only lowercase ASCII letters, numbers, '-', '_', '.', or '/': {tag}"
                    ),
                ));
            }
        }
    }
}

/// 設定ファイルの優先順位に従ってトンネル設定を統合する
fn merge_tunnels(sources: &[LoadedConfigFile]) -> Vec<ResolvedTunnelConfig> {
    let mut tunnels = Vec::new();
    let mut positions = HashMap::<String, usize>::new();
    let base_timeouts = merge_timeout_config(sources).resolve_with_defaults();

    for file in sources {
        for resolved in file.resolved_tunnels(base_timeouts) {
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

/// 設定ファイルの優先順位に従って共通タイムアウト設定を統合する
fn merge_timeout_config(sources: &[LoadedConfigFile]) -> TimeoutConfig {
    sources
        .iter()
        .fold(TimeoutConfig::default(), |timeouts, file| {
            timeouts.apply_overrides(&file.timeouts)
        })
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

/// 編集対象の設定ファイルを読み込む
fn read_config_file_for_edit(
    path: &Path,
    kind: ConfigSourceKind,
    create_if_missing: bool,
) -> Result<LoadedConfigFile, ConfigEditError> {
    if !path.exists() {
        if create_if_missing {
            return Ok(LoadedConfigFile::new(
                ConfigSource::new(kind, path.to_path_buf()),
                Vec::new(),
            ));
        }

        return Err(ConfigEditError::Missing {
            path: path.to_path_buf(),
        });
    }

    let content = fs::read_to_string(path).map_err(|source| ConfigEditError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw =
        toml::from_str::<RawConfigFile>(&content).map_err(|source| ConfigEditError::Parse {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(LoadedConfigFile::with_timeouts(
        ConfigSource::new(kind, path.to_path_buf()),
        raw.timeouts,
        normalize_tunnels(raw.tunnels),
    ))
}

/// 設定ファイルへトンネル一覧を書き込む
fn write_config_file(path: &Path, file: &LoadedConfigFile) -> Result<(), ConfigEditError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| ConfigEditError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let raw = RawConfigFile {
        timeouts: file.timeouts.clone(),
        tunnels: normalize_tunnels(file.tunnels.clone()),
    };
    let content = toml::to_string_pretty(&raw).map_err(|source| ConfigEditError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;

    fs::write(path, content).map_err(|source| ConfigEditError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// TOML から読み込んだタグ一覧を正規化する
fn deserialize_tags<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(|tags| normalize_tags(&tags))
}

/// トンネル一覧のタグを正規化する
fn normalize_tunnels(tunnels: Vec<TunnelConfig>) -> Vec<TunnelConfig> {
    tunnels.into_iter().map(normalize_tunnel).collect()
}

/// トンネル設定のタグを正規化する
fn normalize_tunnel(mut tunnel: TunnelConfig) -> TunnelConfig {
    tunnel.tags = normalize_tags(&tunnel.tags);
    tunnel
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

/// 権限が必要になる可能性があるローカルポートを警告する
fn warn_privileged_local_ports(config: &EffectiveConfig, report: &mut ValidationReport) {
    for resolved in &config.tunnels {
        if (1..1024).contains(&resolved.tunnel.local_port) {
            report.push_warning(ValidationWarning::new(
                resolved.source.clone(),
                Some(resolved.tunnel.id.clone()),
                "local_port below 1024 may require elevated privileges",
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
        assert!(config.tunnels[0].tunnel.tags.is_empty());
    }

    /// local_host 未指定時に既定値が使われることを検証する
    #[test]
    fn tunnel_config_uses_default_local_host_when_omitted() {
        let tunnel = tunnel("db", 15432);

        assert_eq!(tunnel.effective_local_host(), "127.0.0.1");
    }

    /// タイムアウト未指定時に既定値が使われることを検証する
    #[test]
    fn timeout_settings_fall_back_to_defaults_when_omitted() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &path,
            r#"
[[tunnels]]
id = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write configuration");

        let config = load_effective_config(&ConfigPaths::new(None, path)).expect("load config");

        assert_eq!(config.tunnels[0].timeouts, ResolvedTimeoutConfig::default());
    }

    /// 共通タイムアウト設定が各トンネルへ適用されることを検証する
    #[test]
    fn top_level_timeout_settings_apply_to_tunnels() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &path,
            r#"
[timeouts]
connect_timeout_seconds = 20
server_alive_interval_seconds = 40
server_alive_count_max = 4
start_grace_milliseconds = 500

[[tunnels]]
id = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write configuration");

        let config = load_effective_config(&ConfigPaths::new(None, path)).expect("load config");

        assert_eq!(
            config.tunnels[0].timeouts,
            ResolvedTimeoutConfig {
                connect_timeout_seconds: 20,
                server_alive_interval_seconds: 40,
                server_alive_count_max: 4,
                start_grace_milliseconds: 500,
            }
        );
    }

    /// トンネル固有タイムアウト設定が共通設定を上書きすることを検証する
    #[test]
    fn tunnel_timeout_settings_override_top_level_settings() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &path,
            r#"
[timeouts]
connect_timeout_seconds = 20
server_alive_interval_seconds = 40
server_alive_count_max = 4
start_grace_milliseconds = 500

[[tunnels]]
id = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"

[tunnels.timeouts]
connect_timeout_seconds = 5
start_grace_milliseconds = 50
"#,
        )
        .expect("write configuration");

        let config = load_effective_config(&ConfigPaths::new(None, path)).expect("load config");

        assert_eq!(
            config.tunnels[0].timeouts,
            ResolvedTimeoutConfig {
                connect_timeout_seconds: 5,
                server_alive_interval_seconds: 40,
                server_alive_count_max: 4,
                start_grace_milliseconds: 50,
            }
        );
    }

    /// ローカル設定の共通タイムアウトがグローバル設定を上書きすることを検証する
    #[test]
    fn local_top_level_timeout_settings_override_global_settings() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let global_path = temp_dir.path().join("global.toml");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &global_path,
            r#"
[timeouts]
connect_timeout_seconds = 20
server_alive_interval_seconds = 40

[[tunnels]]
id = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write global configuration");
        fs::write(
            &local_path,
            r#"
[timeouts]
connect_timeout_seconds = 10
"#,
        )
        .expect("write local configuration");

        let config = load_effective_config(&ConfigPaths::new(Some(global_path), local_path))
            .expect("load configuration");

        assert_eq!(config.tunnels[0].timeouts.connect_timeout_seconds, 10);
        assert_eq!(config.tunnels[0].timeouts.server_alive_interval_seconds, 40);
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

    /// 特権ポート相当の local_port が警告として扱われることを検証する
    #[test]
    fn validation_warns_privileged_local_ports() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let config = EffectiveConfig::new(
            vec![LoadedConfigFile::new(
                source.clone(),
                vec![tunnel("web", 80)],
            )],
            vec![ResolvedTunnelConfig::new(source, tunnel("web", 80))],
        );

        let report = validate_config(&config);

        assert!(report.is_valid());
        assert!(
            report.warnings.iter().any(|warning| warning.message
                == "local_port below 1024 may require elevated privileges")
        );
    }

    /// タグ付き設定が TOML として保存し、同じ内容で読み戻せることを検証する
    #[test]
    fn tagged_tunnel_round_trips_as_toml() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        let mut tunnel = tunnel("db", 15432);
        tunnel.tags = vec!["Dev".to_owned(), "project-a".to_owned()];

        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel)
            .expect("add tagged tunnel");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert_eq!(
            loaded.tunnels[0].tags,
            vec!["dev".to_owned(), "project-a".to_owned()]
        );
    }

    /// タイムアウト設定を TOML として保存し、同じ内容で読み戻せることを検証する
    #[test]
    fn timeout_settings_round_trip_as_toml() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        let mut tunnel = tunnel("db", 15432);
        tunnel.timeouts.connect_timeout_seconds = Some(5);
        tunnel.timeouts.start_grace_milliseconds = Some(50);

        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel).expect("add tunnel");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");
        let content = fs::read_to_string(path).expect("read configuration content");

        assert_eq!(loaded.tunnels[0].timeouts.connect_timeout_seconds, Some(5));
        assert_eq!(
            loaded.tunnels[0].timeouts.start_grace_milliseconds,
            Some(50)
        );
        assert!(content.contains("[[tunnels]]"));
        assert!(content.contains("[tunnels.timeouts]"));
    }

    /// 設定編集時に共通タイムアウト設定が保持されることを検証する
    #[test]
    fn add_tunnel_preserves_top_level_timeout_settings() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &path,
            r#"
[timeouts]
connect_timeout_seconds = 20

[[tunnels]]
id = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write configuration");

        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("cache", 16379))
            .expect("add tunnel");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert_eq!(loaded.timeouts.connect_timeout_seconds, Some(20));
        assert_eq!(loaded.tunnels.len(), 2);
    }

    /// タグなし設定では保存時に tags を出力しないことを検証する
    #[test]
    fn empty_tags_are_omitted_when_serializing() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");

        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add tunnel");
        let content = fs::read_to_string(path).expect("read configuration");

        assert!(!content.contains("tags"));
    }

    /// タグの正規化と検証ルールを検証する
    #[test]
    fn tags_are_normalized_and_validated() {
        assert_eq!(normalize_tag(" Dev "), "dev");
        assert!(tag_is_valid("project-a"));
        assert!(tag_is_valid("client/foo"));
        assert!(!tag_is_valid(""));
        assert!(!tag_is_valid("project a"));
        assert!(!tag_is_valid("案件"));
    }

    /// タグ指定が AND 条件でトンネルを絞り込むことを検証する
    #[test]
    fn filter_tunnels_by_tags_matches_all_tags() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let mut dev_db = tunnel("dev-db", 15432);
        dev_db.tags = vec!["dev".to_owned(), "project-a".to_owned()];
        let mut prod_db = tunnel("prod-db", 25432);
        prod_db.tags = vec!["prod".to_owned(), "project-a".to_owned()];
        let tunnels = vec![
            ResolvedTunnelConfig::new(source.clone(), dev_db),
            ResolvedTunnelConfig::new(source, prod_db),
        ];

        let matched = filter_tunnels_by_tags(&tunnels, &["dev".to_owned(), "project-a".to_owned()]);

        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].tunnel.id, "dev-db");
    }

    /// 未正規化のトンネルタグでもタグ指定に一致することを検証する
    #[test]
    fn tunnel_matches_tags_compares_without_requiring_normalized_tunnel_tags() {
        let mut tunnel = tunnel("dev-db", 15432);
        tunnel.tags = vec![" Dev ".to_owned(), "PROJECT-A".to_owned()];

        let matched = tunnel_matches_tags(&tunnel, &["dev".to_owned(), "project-a".to_owned()]);

        assert!(matched);
    }

    /// 存在しない設定ファイルへトンネルを追加できることを検証する
    #[test]
    fn add_tunnel_creates_missing_config_file() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");

        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add tunnel");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert_eq!(loaded.tunnels.len(), 1);
        assert_eq!(loaded.tunnels[0].id, "db");
    }

    /// 同一設定ファイル内の ID 重複が追加時に拒否されることを検証する
    #[test]
    fn add_tunnel_rejects_duplicate_id() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add tunnel");

        let result = add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 25432));

        assert!(matches!(result, Err(ConfigEditError::DuplicateId { .. })));
    }

    /// 指定 ID のトンネルが設定ファイルから削除されることを検証する
    #[test]
    fn remove_tunnel_removes_matching_id() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add first tunnel");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("cache", 16379))
            .expect("add second tunnel");

        remove_tunnel_from_config_file(&path, ConfigSourceKind::Local, "db")
            .expect("remove tunnel");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert_eq!(loaded.tunnels.len(), 1);
        assert_eq!(loaded.tunnels[0].id, "cache");
    }

    /// 指定 ID のトンネルが設定ファイル内で更新されることを検証する
    #[test]
    fn update_tunnel_replaces_matching_id() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add first tunnel");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("cache", 16379))
            .expect("add second tunnel");

        let mut updated = tunnel("dev-db", 25432);
        updated.description = Some("Development database".to_owned());
        update_tunnel_in_config_file(&path, ConfigSourceKind::Local, "db", updated)
            .expect("update tunnel");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert_eq!(loaded.tunnels.len(), 2);
        assert_eq!(loaded.tunnels[0].id, "dev-db");
        assert_eq!(
            loaded.tunnels[0].description.as_deref(),
            Some("Development database")
        );
        assert_eq!(loaded.tunnels[1].id, "cache");
    }

    /// 更新後 ID が同一設定ファイル内で重複する場合に拒否されることを検証する
    #[test]
    fn update_tunnel_rejects_duplicate_id() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add first tunnel");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("cache", 16379))
            .expect("add second tunnel");

        let result = update_tunnel_in_config_file(
            &path,
            ConfigSourceKind::Local,
            "db",
            tunnel("cache", 25432),
        );

        assert!(matches!(result, Err(ConfigEditError::DuplicateId { .. })));
    }

    /// テスト用のトンネル設定を生成する
    fn tunnel(id: &str, local_port: u16) -> TunnelConfig {
        TunnelConfig {
            id: id.to_owned(),
            description: None,
            tags: Vec::new(),
            local_host: None,
            local_port,
            remote_host: "db.internal".to_owned(),
            remote_port: 5432,
            ssh_user: "user".to_owned(),
            ssh_host: "bastion.example.com".to_owned(),
            ssh_port: None,
            identity_file: None,
            timeouts: TimeoutConfig::default(),
        }
    }
}
