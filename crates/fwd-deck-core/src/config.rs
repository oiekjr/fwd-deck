use std::{
    collections::{HashMap, HashSet},
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
const LOCAL_OVERRIDE_MARKER: &str = ".override";
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
    pub local_override: PathBuf,
}

impl ConfigPaths {
    /// 設定ファイルの位置を初期化する
    pub fn new(global: Option<PathBuf>, local: PathBuf) -> Self {
        let local_override = local_override_config_path(&local);

        Self {
            global,
            local,
            local_override,
        }
    }
}

/// 設定ファイルの種類を表現する
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    pub name: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
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

/// local 設定へ適用するトンネル単位の上書きを表現する
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TunnelConfigOverride {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "TimeoutConfig::is_empty")]
    pub timeouts: TimeoutConfig,
}

impl TunnelConfigOverride {
    /// name だけを持つ空の上書きを初期化する
    pub fn new(name: String) -> Self {
        Self {
            name,
            enabled: None,
            description: None,
            tags: None,
            local_host: None,
            local_port: None,
            remote_host: None,
            remote_port: None,
            ssh_user: None,
            ssh_host: None,
            ssh_port: None,
            identity_file: None,
            timeouts: TimeoutConfig::default(),
        }
    }

    /// 実際に上書きする項目が含まれるかを判定する
    pub fn has_overrides(&self) -> bool {
        self.enabled.is_some()
            || self.description.is_some()
            || self.tags.is_some()
            || self.local_host.is_some()
            || self.local_port.is_some()
            || self.remote_host.is_some()
            || self.remote_port.is_some()
            || self.ssh_user.is_some()
            || self.ssh_host.is_some()
            || self.ssh_port.is_some()
            || self.identity_file.is_some()
            || !self.timeouts.is_empty()
    }

    /// 基準トンネルへ上書き値を適用する
    pub fn apply_to(&self, base: &TunnelConfig) -> TunnelConfig {
        TunnelConfig {
            name: base.name.clone(),
            enabled: self.enabled.unwrap_or(base.enabled),
            description: self
                .description
                .clone()
                .or_else(|| base.description.clone()),
            tags: self.tags.clone().unwrap_or_else(|| base.tags.clone()),
            local_host: self.local_host.clone().or_else(|| base.local_host.clone()),
            local_port: self.local_port.unwrap_or(base.local_port),
            remote_host: self
                .remote_host
                .clone()
                .unwrap_or_else(|| base.remote_host.clone()),
            remote_port: self.remote_port.unwrap_or(base.remote_port),
            ssh_user: self
                .ssh_user
                .clone()
                .unwrap_or_else(|| base.ssh_user.clone()),
            ssh_host: self
                .ssh_host
                .clone()
                .unwrap_or_else(|| base.ssh_host.clone()),
            ssh_port: self.ssh_port.or(base.ssh_port),
            identity_file: self
                .identity_file
                .clone()
                .or_else(|| base.identity_file.clone()),
            timeouts: base.timeouts.apply_overrides(&self.timeouts),
        }
    }
}

/// 読み込み済み local override ファイルを表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfigOverrideFile {
    pub source: ConfigSource,
    pub tunnels: Vec<TunnelConfigOverride>,
}

impl LoadedConfigOverrideFile {
    /// 読み込み済み local override ファイルを初期化する
    pub fn new(source: ConfigSource, tunnels: Vec<TunnelConfigOverride>) -> Self {
        Self { source, tunnels }
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
}

/// 統合後のトンネル設定を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTunnelConfig {
    pub source: ConfigSource,
    pub override_source: Option<ConfigSource>,
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
            override_source: None,
            tunnel,
            timeouts,
        }
    }

    /// 上書き元を含む統合後トンネル設定を初期化する
    fn new_with_override(
        source: ConfigSource,
        override_source: Option<ConfigSource>,
        tunnel: TunnelConfig,
        timeouts: ResolvedTimeoutConfig,
    ) -> Self {
        Self {
            source,
            override_source,
            tunnel,
            timeouts,
        }
    }

    /// local override が適用されているかを判定する
    pub fn is_overridden(&self) -> bool {
        self.override_source.is_some()
    }
}

/// 統合済み設定全体を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub sources: Vec<LoadedConfigFile>,
    pub local_override: Option<LoadedConfigOverrideFile>,
    pub configured_tunnels: Vec<ResolvedTunnelConfig>,
    pub tunnels: Vec<ResolvedTunnelConfig>,
}

impl EffectiveConfig {
    /// 統合済み設定を初期化する
    pub fn new(sources: Vec<LoadedConfigFile>, tunnels: Vec<ResolvedTunnelConfig>) -> Self {
        let configured_tunnels = if sources.is_empty() {
            tunnels.clone()
        } else {
            merge_tunnels(&sources, None)
        };

        Self {
            sources,
            local_override: None,
            configured_tunnels,
            tunnels,
        }
    }

    /// local override を含む統合済み設定を初期化する
    fn with_local_override(
        sources: Vec<LoadedConfigFile>,
        local_override: Option<LoadedConfigOverrideFile>,
        configured_tunnels: Vec<ResolvedTunnelConfig>,
    ) -> Self {
        let tunnels = configured_tunnels
            .iter()
            .filter(|resolved| resolved.tunnel.enabled)
            .cloned()
            .collect();

        Self {
            sources,
            local_override,
            configured_tunnels,
            tunnels,
        }
    }

    /// 設定ファイルが読み込まれているかを判定する
    pub fn has_sources(&self) -> bool {
        !self.sources.is_empty() || self.local_override.is_some()
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

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawConfigOverrideFile {
    #[serde(default)]
    tunnels: Vec<TunnelConfigOverride>,
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
        "Tunnel name already exists in configuration file: {name} ({})",
        format_path_for_display(.path)
    )]
    DuplicateName { path: PathBuf, name: String },
    #[error(
        "Local port already exists in configuration file: {local_port} ({existing_name}, {})",
        format_path_for_display(.path)
    )]
    DuplicateLocalPort {
        path: PathBuf,
        local_port: u16,
        existing_name: String,
    },
    #[error(
        "Tunnel name was not found in configuration file: {name} ({})",
        format_path_for_display(.path)
    )]
    NotFound { path: PathBuf, name: String },
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
    #[error(
        "Failed to remove configuration file: {}: {source}",
        format_path_for_display(.path)
    )]
    Remove {
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
    pub tunnel_name: Option<String>,
    pub message: String,
}

impl ValidationError {
    /// 検証エラーを初期化する
    pub fn new(
        source: ConfigSource,
        tunnel_name: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source,
            tunnel_name,
            message: message.into(),
        }
    }
}

/// 設定検証で検出した注意事項を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationWarning {
    pub source: ConfigSource,
    pub tunnel_name: Option<String>,
    pub message: String,
}

impl ValidationWarning {
    /// 検証警告を初期化する
    pub fn new(
        source: ConfigSource,
        tunnel_name: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source,
            tunnel_name,
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

/// local 設定ファイルから対応する override ファイルパスを生成する
pub fn local_override_config_path(local_path: &Path) -> PathBuf {
    let Some(file_name) = local_path.file_name() else {
        return local_path.join("fwd-deck.override.toml");
    };
    let mut override_file_name = local_path.file_stem().unwrap_or(file_name).to_os_string();
    override_file_name.push(LOCAL_OVERRIDE_MARKER);

    if let Some(extension) = local_path.extension() {
        override_file_name.push(".");
        override_file_name.push(extension);
    } else {
        override_file_name.push(".toml");
    }

    local_path.with_file_name(override_file_name)
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

/// 指定された local override ファイルを読み込む
pub fn read_config_override_file(
    path: &Path,
) -> Result<Option<LoadedConfigOverrideFile>, ConfigLoadError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|source| ConfigLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let raw = toml::from_str::<RawConfigOverrideFile>(&content).map_err(|source| {
        ConfigLoadError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })?;

    Ok(Some(LoadedConfigOverrideFile::new(
        ConfigSource::new(ConfigSourceKind::Local, path.to_path_buf()),
        normalize_tunnel_overrides(raw.tunnels),
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

    if file
        .tunnels
        .iter()
        .any(|existing| existing.name == tunnel.name)
    {
        return Err(ConfigEditError::DuplicateName {
            path: path.to_path_buf(),
            name: tunnel.name,
        });
    }

    if let Some(existing) = file
        .tunnels
        .iter()
        .find(|existing| existing.local_port == tunnel.local_port)
    {
        return Err(ConfigEditError::DuplicateLocalPort {
            path: path.to_path_buf(),
            local_port: tunnel.local_port,
            existing_name: existing.name.clone(),
        });
    }

    file.tunnels.push(tunnel);
    write_config_file(path, &file)?;

    Ok(file)
}

/// 指定された設定ファイルへ複数トンネル設定をまとめて追加する
pub fn add_tunnels_to_config_file(
    path: &Path,
    kind: ConfigSourceKind,
    tunnels: Vec<TunnelConfig>,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let mut file = read_config_file_for_edit(path, kind, true)?;
    let tunnels = normalize_tunnels(tunnels);
    let mut existing_names = HashMap::<String, ()>::new();
    let mut existing_local_ports = HashMap::<u16, String>::new();

    for existing in &file.tunnels {
        existing_names.insert(existing.name.clone(), ());
        existing_local_ports
            .entry(existing.local_port)
            .or_insert_with(|| existing.name.clone());
    }

    for tunnel in &tunnels {
        if existing_names.contains_key(&tunnel.name) {
            return Err(ConfigEditError::DuplicateName {
                path: path.to_path_buf(),
                name: tunnel.name.clone(),
            });
        }

        if let Some(existing_name) = existing_local_ports.get(&tunnel.local_port) {
            return Err(ConfigEditError::DuplicateLocalPort {
                path: path.to_path_buf(),
                local_port: tunnel.local_port,
                existing_name: existing_name.clone(),
            });
        }

        existing_names.insert(tunnel.name.clone(), ());
        existing_local_ports.insert(tunnel.local_port, tunnel.name.clone());
    }

    file.tunnels.extend(tunnels);
    write_config_file(path, &file)?;

    Ok(file)
}

/// 指定された設定ファイルからトンネル設定を削除する
pub fn remove_tunnel_from_config_file(
    path: &Path,
    kind: ConfigSourceKind,
    name: &str,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let mut file = read_config_file_for_edit(path, kind, false)?;
    let Some(position) = file.tunnels.iter().position(|tunnel| tunnel.name == name) else {
        return Err(ConfigEditError::NotFound {
            path: path.to_path_buf(),
            name: name.to_owned(),
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
    name: &str,
    tunnel: TunnelConfig,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let mut file = read_config_file_for_edit(path, kind, false)?;
    let tunnel = normalize_tunnel(tunnel);
    let Some(position) = file
        .tunnels
        .iter()
        .position(|existing| existing.name == name)
    else {
        return Err(ConfigEditError::NotFound {
            path: path.to_path_buf(),
            name: name.to_owned(),
        });
    };

    if file
        .tunnels
        .iter()
        .enumerate()
        .any(|(index, existing)| index != position && existing.name == tunnel.name)
    {
        return Err(ConfigEditError::DuplicateName {
            path: path.to_path_buf(),
            name: tunnel.name,
        });
    }

    if let Some(existing) = file
        .tunnels
        .iter()
        .enumerate()
        .find_map(|(index, existing)| {
            (index != position && existing.local_port == tunnel.local_port).then_some(existing)
        })
    {
        return Err(ConfigEditError::DuplicateLocalPort {
            path: path.to_path_buf(),
            local_port: tunnel.local_port,
            existing_name: existing.name.clone(),
        });
    }

    file.tunnels[position] = tunnel;
    write_config_file(path, &file)?;

    Ok(file)
}

/// local override ファイルへトンネル上書きを追加または更新する
pub fn upsert_tunnel_override_in_config_file(
    path: &Path,
    tunnel_override: TunnelConfigOverride,
) -> Result<Option<LoadedConfigOverrideFile>, ConfigEditError> {
    let mut file = read_config_override_file_for_edit(path, true)?;
    let tunnel_override = normalize_tunnel_override(tunnel_override);
    let position = unique_tunnel_override_position(&file, path, &tunnel_override.name)?;

    if tunnel_override.has_overrides() {
        if let Some(position) = position {
            file.tunnels[position] = tunnel_override;
        } else {
            file.tunnels.push(tunnel_override);
        }
    } else if let Some(position) = position {
        file.tunnels.remove(position);
    }

    write_or_remove_config_override_file(path, &file)
}

/// local override ファイルから指定トンネルの上書きを解除する
pub fn remove_tunnel_override_from_config_file(
    path: &Path,
    name: &str,
) -> Result<Option<LoadedConfigOverrideFile>, ConfigEditError> {
    let mut file = read_config_override_file_for_edit(path, false)?;
    let Some(position) = unique_tunnel_override_position(&file, path, name)? else {
        return Err(ConfigEditError::NotFound {
            path: path.to_path_buf(),
            name: name.to_owned(),
        });
    };

    file.tunnels.remove(position);
    write_or_remove_config_override_file(path, &file)
}

/// 本体のトンネル更新と override の name 追従を実行する
pub fn update_tunnel_and_override_name_in_config_files(
    config_path: &Path,
    override_path: &Path,
    kind: ConfigSourceKind,
    current_name: &str,
    tunnel: TunnelConfig,
) -> Result<LoadedConfigFile, ConfigEditError> {
    let next_name = tunnel.name.clone();

    if kind == ConfigSourceKind::Local && current_name != next_name {
        validate_tunnel_override_rename(override_path, current_name, &next_name)?;
    }

    let file = update_tunnel_in_config_file(config_path, kind, current_name, tunnel)?;

    if kind == ConfigSourceKind::Local && current_name != next_name {
        rename_tunnel_override_if_exists(override_path, current_name, &next_name)?;
    }

    Ok(file)
}

/// 本体と override から指定トンネルを削除する
pub fn remove_tunnel_and_override_from_config_files(
    config_path: &Path,
    override_path: &Path,
    kind: ConfigSourceKind,
    name: &str,
) -> Result<LoadedConfigFile, ConfigEditError> {
    if kind == ConfigSourceKind::Local {
        validate_tunnel_override_removal(override_path, name)?;
    }

    let file = remove_tunnel_from_config_file(config_path, kind, name)?;

    if kind == ConfigSourceKind::Local {
        remove_tunnel_override_if_exists(override_path, name)?;
    }

    Ok(file)
}

/// グローバル設定とローカル設定を統合して読み込む
pub fn load_effective_config(paths: &ConfigPaths) -> Result<EffectiveConfig, ConfigLoadError> {
    let sources = read_existing_config_files(paths)?;
    let local_override = read_config_override_file(&paths.local_override)?;
    let configured_tunnels = merge_tunnels(&sources, local_override.as_ref());

    Ok(EffectiveConfig::with_local_override(
        sources,
        local_override,
        configured_tunnels,
    ))
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

/// 指定タグを1つでも持つ統合済みトンネル設定を除外する
pub fn filter_tunnels_excluding_tags<'a>(
    tunnels: &'a [ResolvedTunnelConfig],
    excluded_tags: &[String],
) -> Vec<&'a ResolvedTunnelConfig> {
    let excluded_tags = normalize_tags(excluded_tags);

    tunnels
        .iter()
        .filter(|resolved| !tunnel_matches_any_normalized_tag(&resolved.tunnel, &excluded_tags))
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

/// 正規化済みタグ条件のいずれかでトンネルを照合する
fn tunnel_matches_any_normalized_tag(tunnel: &TunnelConfig, target_tags: &[String]) -> bool {
    if target_tags.is_empty() {
        return false;
    }

    target_tags.iter().any(|target| {
        tunnel
            .tags
            .iter()
            .any(|tag| tag_matches_normalized(tag, target))
    })
}

/// 正規化済み条件とトンネル側タグを比較する
fn tag_matches_normalized(tag: &str, normalized_required: &str) -> bool {
    tag.trim().eq_ignore_ascii_case(normalized_required)
}

/// 設定内容の意味的な不備を検証する
pub fn validate_config(config: &EffectiveConfig) -> ValidationReport {
    let mut report = ValidationReport::valid();

    validate_duplicate_names(config, &mut report);
    validate_config_override(config, &mut report);
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
    for (source, tunnel) in source_tunnels(config) {
        if let Some(local_host) = &tunnel.local_host {
            validate_local_host(source, tunnel, local_host, report);
        }
    }
}

/// ローカル側の bind address として扱える値かを検証する
fn validate_local_host(
    source: &ConfigSource,
    tunnel: &TunnelConfig,
    local_host: &str,
    report: &mut ValidationReport,
) {
    if local_host.trim().is_empty() {
        report.push(ValidationError::new(
            source.clone(),
            Some(tunnel.name.clone()),
            "local_host cannot be empty",
        ));
        return;
    }

    if local_host.chars().any(char::is_whitespace) {
        report.push(ValidationError::new(
            source.clone(),
            Some(tunnel.name.clone()),
            "local_host cannot contain whitespace",
        ));
    }
}

/// タグが許可された形式で記述されているかを検証する
fn validate_tags(config: &EffectiveConfig, report: &mut ValidationReport) {
    for (source, tunnel) in source_tunnels(config) {
        for tag in &tunnel.tags {
            if !tag_is_valid(tag) {
                report.push(ValidationError::new(
                    source.clone(),
                    Some(tunnel.name.clone()),
                    format!(
                        "tag must contain only lowercase ASCII letters, numbers, '-', '_', '.', or '/': {tag}"
                    ),
                ));
            }
        }
    }
}

/// 設定ファイルの優先順位に従ってトンネル設定を統合する
fn merge_tunnels(
    sources: &[LoadedConfigFile],
    local_override: Option<&LoadedConfigOverrideFile>,
) -> Vec<ResolvedTunnelConfig> {
    let tunnel_count = sources.iter().map(|file| file.tunnels.len()).sum();
    let mut tunnels = Vec::with_capacity(tunnel_count);
    let base_timeouts = merge_timeout_config(sources).resolve_with_defaults();
    let local_overrides_by_name = index_local_overrides_by_name(local_override);

    for file in sources {
        for base_tunnel in &file.tunnels {
            let applied_override = if file.source.kind == ConfigSourceKind::Local {
                local_overrides_by_name
                    .get(base_tunnel.name.as_str())
                    .copied()
            } else {
                None
            };
            let tunnel = applied_override
                .map(|tunnel_override| tunnel_override.apply_to(base_tunnel))
                .unwrap_or_else(|| base_tunnel.clone());
            let timeouts = tunnel.timeouts.resolve_with_base(base_timeouts);
            let override_source = applied_override
                .and(local_override)
                .map(|file| file.source.clone());

            tunnels.push(ResolvedTunnelConfig::new_with_override(
                file.source.clone(),
                override_source,
                tunnel,
                timeouts,
            ));
        }
    }

    tunnels
}

/// local override を name で検索できる形式へ変換する
fn index_local_overrides_by_name(
    local_override: Option<&LoadedConfigOverrideFile>,
) -> HashMap<&str, &TunnelConfigOverride> {
    let Some(local_override) = local_override else {
        return HashMap::new();
    };
    let mut overrides = HashMap::with_capacity(local_override.tunnels.len());

    for tunnel_override in &local_override.tunnels {
        if tunnel_override.has_overrides() {
            overrides
                .entry(tunnel_override.name.as_str())
                .or_insert(tunnel_override);
        }
    }

    overrides
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
            return Ok(LoadedConfigFile::with_timeouts(
                ConfigSource::new(kind, path.to_path_buf()),
                default_timeout_config_for_new_file(),
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

/// 編集対象の local override ファイルを読み込む
fn read_config_override_file_for_edit(
    path: &Path,
    create_if_missing: bool,
) -> Result<LoadedConfigOverrideFile, ConfigEditError> {
    if !path.exists() {
        if create_if_missing {
            return Ok(LoadedConfigOverrideFile::new(
                ConfigSource::new(ConfigSourceKind::Local, path.to_path_buf()),
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
    let raw = toml::from_str::<RawConfigOverrideFile>(&content).map_err(|source| {
        ConfigEditError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })?;

    Ok(LoadedConfigOverrideFile::new(
        ConfigSource::new(ConfigSourceKind::Local, path.to_path_buf()),
        normalize_tunnel_overrides(raw.tunnels),
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

/// local override ファイルを書き込む
fn write_config_override_file(
    path: &Path,
    file: &LoadedConfigOverrideFile,
) -> Result<(), ConfigEditError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| ConfigEditError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let raw = RawConfigOverrideFile {
        tunnels: normalize_tunnel_overrides(file.tunnels.clone()),
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

/// 空になった local override ファイルを削除し、それ以外を書き込む
fn write_or_remove_config_override_file(
    path: &Path,
    file: &LoadedConfigOverrideFile,
) -> Result<Option<LoadedConfigOverrideFile>, ConfigEditError> {
    if file.tunnels.is_empty() {
        if path.exists() {
            fs::remove_file(path).map_err(|source| ConfigEditError::Remove {
                path: path.to_path_buf(),
                source,
            })?;
        }

        return Ok(None);
    }

    write_config_override_file(path, file)?;
    Ok(Some(file.clone()))
}

/// local override が存在する場合だけ name を変更する
fn rename_tunnel_override_if_exists(
    path: &Path,
    current_name: &str,
    next_name: &str,
) -> Result<(), ConfigEditError> {
    if !path.exists() {
        return Ok(());
    }

    let mut file = read_config_override_file_for_edit(path, false)?;
    let Some(position) = unique_tunnel_override_position(&file, path, current_name)? else {
        return Ok(());
    };

    if file
        .tunnels
        .iter()
        .enumerate()
        .any(|(index, existing)| index != position && existing.name == next_name)
    {
        return Err(ConfigEditError::DuplicateName {
            path: path.to_path_buf(),
            name: next_name.to_owned(),
        });
    }

    file.tunnels[position].name = next_name.to_owned();
    write_config_override_file(path, &file)
}

/// local override が存在する場合だけ指定 name を削除する
fn remove_tunnel_override_if_exists(path: &Path, name: &str) -> Result<(), ConfigEditError> {
    if !path.exists() {
        return Ok(());
    }

    let mut file = read_config_override_file_for_edit(path, false)?;
    let Some(position) = unique_tunnel_override_position(&file, path, name)? else {
        return Ok(());
    };

    file.tunnels.remove(position);
    write_or_remove_config_override_file(path, &file)?;
    Ok(())
}

/// local override 内で指定 name が一意な位置を取得する
fn unique_tunnel_override_position(
    file: &LoadedConfigOverrideFile,
    path: &Path,
    name: &str,
) -> Result<Option<usize>, ConfigEditError> {
    let mut positions = file
        .tunnels
        .iter()
        .enumerate()
        .filter_map(|(index, existing)| (existing.name == name).then_some(index));
    let position = positions.next();

    if positions.next().is_some() {
        return Err(ConfigEditError::DuplicateName {
            path: path.to_path_buf(),
            name: name.to_owned(),
        });
    }

    Ok(position)
}

/// 本体更新前に local override の name 追従可否を検証する
fn validate_tunnel_override_rename(
    path: &Path,
    current_name: &str,
    next_name: &str,
) -> Result<(), ConfigEditError> {
    if !path.exists() {
        return Ok(());
    }

    let file = read_config_override_file_for_edit(path, false)?;
    let Some(position) = unique_tunnel_override_position(&file, path, current_name)? else {
        return Ok(());
    };

    if file
        .tunnels
        .iter()
        .enumerate()
        .any(|(index, existing)| index != position && existing.name == next_name)
    {
        return Err(ConfigEditError::DuplicateName {
            path: path.to_path_buf(),
            name: next_name.to_owned(),
        });
    }

    Ok(())
}

/// 本体削除前に対応する local override が一意か検証する
fn validate_tunnel_override_removal(path: &Path, name: &str) -> Result<(), ConfigEditError> {
    if !path.exists() {
        return Ok(());
    }

    let file = read_config_override_file_for_edit(path, false)?;
    unique_tunnel_override_position(&file, path, name)?;
    Ok(())
}

/// 新規作成する設定ファイルに出力する既定タイムアウト設定を生成する
fn default_timeout_config_for_new_file() -> TimeoutConfig {
    TimeoutConfig {
        connect_timeout_seconds: Some(DEFAULT_CONNECT_TIMEOUT_SECONDS),
        server_alive_interval_seconds: Some(DEFAULT_SERVER_ALIVE_INTERVAL_SECONDS),
        server_alive_count_max: Some(DEFAULT_SERVER_ALIVE_COUNT_MAX),
        start_grace_milliseconds: Some(DEFAULT_START_GRACE_MILLISECONDS),
    }
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

/// トンネル上書き一覧のタグを正規化する
fn normalize_tunnel_overrides(
    tunnel_overrides: Vec<TunnelConfigOverride>,
) -> Vec<TunnelConfigOverride> {
    tunnel_overrides
        .into_iter()
        .map(normalize_tunnel_override)
        .collect()
}

/// トンネル上書きのタグを正規化する
fn normalize_tunnel_override(mut tunnel_override: TunnelConfigOverride) -> TunnelConfigOverride {
    tunnel_override.tags = tunnel_override.tags.map(|tags| normalize_tags(&tags));
    tunnel_override
}

/// bool 設定の未指定時に有効を補う
fn default_true() -> bool {
    true
}

/// TOML 出力を省略できる有効設定かを判定する
fn is_true(value: &bool) -> bool {
    *value
}

/// 読み込み元ファイルに含まれる全トンネル設定を取得する
fn source_tunnels(
    config: &EffectiveConfig,
) -> impl Iterator<Item = (&ConfigSource, &TunnelConfig)> {
    let use_configured_tunnels = config.sources.is_empty();

    config
        .sources
        .iter()
        .flat_map(|file| {
            file.tunnels
                .iter()
                .map(move |tunnel| (&file.source, tunnel))
        })
        .chain(
            config
                .configured_tunnels
                .iter()
                .filter(move |_| use_configured_tunnels)
                .map(|resolved| (&resolved.source, &resolved.tunnel)),
        )
}

/// 同一設定ファイル内の name 重複を検証する
fn validate_duplicate_names(config: &EffectiveConfig, report: &mut ValidationReport) {
    for file in &config.sources {
        let mut counts = HashMap::<&str, usize>::new();

        for tunnel in &file.tunnels {
            *counts.entry(tunnel.name.as_str()).or_default() += 1;
        }

        for (name, count) in counts {
            if count > 1 {
                report.push(ValidationError::new(
                    file.source.clone(),
                    Some(name.to_owned()),
                    "Duplicate name in the same configuration file",
                ));
            }
        }
    }
}

/// local override の name が一意かつ本体に対応するかを検証する
fn validate_config_override(config: &EffectiveConfig, report: &mut ValidationReport) {
    let Some(local_override) = &config.local_override else {
        return;
    };
    let local_names = config
        .sources
        .iter()
        .filter(|file| file.source.kind == ConfigSourceKind::Local)
        .flat_map(|file| file.tunnels.iter().map(|tunnel| tunnel.name.as_str()))
        .collect::<std::collections::HashSet<_>>();
    let mut counts = HashMap::<&str, usize>::new();

    for tunnel_override in &local_override.tunnels {
        *counts.entry(tunnel_override.name.as_str()).or_default() += 1;

        if tunnel_override.name.trim().is_empty() {
            report.push(ValidationError::new(
                local_override.source.clone(),
                Some(tunnel_override.name.clone()),
                "Override name cannot be empty",
            ));
        } else if !local_names.contains(tunnel_override.name.as_str()) {
            report.push_warning(ValidationWarning::new(
                local_override.source.clone(),
                Some(tunnel_override.name.clone()),
                "Override name was not found in local configuration; entry was ignored",
            ));
        } else {
            validate_tunnel_override_fields(&local_override.source, tunnel_override, report);
        }
    }

    for (name, count) in counts {
        if count > 1 {
            report.push(ValidationError::new(
                local_override.source.clone(),
                Some(name.to_owned()),
                "Duplicate name in local override configuration file",
            ));
        }
    }
}

/// local override で指定された値自体の制約を検証する
fn validate_tunnel_override_fields(
    source: &ConfigSource,
    tunnel_override: &TunnelConfigOverride,
    report: &mut ValidationReport,
) {
    let name = &tunnel_override.name;

    if let Some(local_host) = tunnel_override.local_host.as_deref() {
        if local_host.trim().is_empty() {
            push_override_error(source, name, "local_host cannot be empty", report);
        } else if local_host.chars().any(char::is_whitespace) {
            push_override_error(source, name, "local_host cannot contain whitespace", report);
        }
    }

    for (field_name, value) in [
        ("remote_host", tunnel_override.remote_host.as_deref()),
        ("ssh_user", tunnel_override.ssh_user.as_deref()),
        ("ssh_host", tunnel_override.ssh_host.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            push_override_error(
                source,
                name,
                &format!("{field_name} cannot be empty"),
                report,
            );
        }
    }

    for (field_name, port) in [
        ("local_port", tunnel_override.local_port),
        ("remote_port", tunnel_override.remote_port),
    ] {
        if port == Some(0) {
            push_override_error(
                source,
                name,
                &format!("{field_name} must be greater than or equal to 1"),
                report,
            );
        }
    }

    if let Some(tags) = &tunnel_override.tags {
        for tag in tags {
            if !tag_is_valid(tag) {
                push_override_error(
                    source,
                    name,
                    &format!(
                        "tag must contain only lowercase ASCII letters, numbers, '-', '_', '.', or '/': {tag}"
                    ),
                    report,
                );
            }
        }
    }

    if tunnel_override
        .local_port
        .is_some_and(|port| (1..1024).contains(&port))
    {
        report.push_warning(ValidationWarning::new(
            source.clone(),
            Some(name.clone()),
            "local_port below 1024 may require elevated privileges",
        ));
    }
}

/// local override の検証エラーを追加する
fn push_override_error(
    source: &ConfigSource,
    name: &str,
    message: &str,
    report: &mut ValidationReport,
) {
    report.push(ValidationError::new(
        source.clone(),
        Some(name.to_owned()),
        message,
    ));
}

/// 必須項目が空文字列ではないことを検証する
fn validate_required_fields(config: &EffectiveConfig, report: &mut ValidationReport) {
    for (source, tunnel) in source_tunnels(config) {
        for (field_name, value) in required_string_fields(tunnel) {
            validate_non_empty(source, tunnel, field_name, value, report);
        }
    }
}

/// 空文字列を禁止する項目を取得する
fn required_string_fields(tunnel: &TunnelConfig) -> [(&'static str, &str); 4] {
    [
        ("name", tunnel.name.as_str()),
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
            Some(tunnel.name.clone()),
            format!("{field_name} cannot be empty"),
        ));
    }
}

/// ポート番号が有効範囲であることを検証する
fn validate_ports(config: &EffectiveConfig, report: &mut ValidationReport) {
    for (source, tunnel) in source_tunnels(config) {
        for (field_name, port) in port_fields(tunnel) {
            validate_non_zero_port(source, tunnel, field_name, port, report);
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
    source: &ConfigSource,
    tunnel: &TunnelConfig,
    field_name: &str,
    port: u16,
    report: &mut ValidationReport,
) {
    if port == 0 {
        report.push(ValidationError::new(
            source.clone(),
            Some(tunnel.name.clone()),
            format!("{field_name} must be greater than or equal to 1"),
        ));
    }
}

/// 同一設定ファイル内のローカルポート重複を検証する
fn validate_duplicate_local_ports(config: &EffectiveConfig, report: &mut ValidationReport) {
    let mut raw_duplicate_ports = HashSet::<(&ConfigSource, u16)>::new();

    for file in &config.sources {
        let mut ports = HashMap::<u16, &TunnelConfig>::new();

        for tunnel in &file.tunnels {
            if let Some(existing) = ports.insert(tunnel.local_port, tunnel) {
                raw_duplicate_ports.insert((&file.source, tunnel.local_port));
                report.push(ValidationError::new(
                    file.source.clone(),
                    Some(tunnel.name.clone()),
                    format!(
                        "local_port {} duplicates {}",
                        tunnel.local_port, existing.name
                    ),
                ));
            }
        }
    }

    let mut effective_ports = HashMap::<&ConfigSource, HashMap<u16, &ResolvedTunnelConfig>>::new();

    for resolved in &config.configured_tunnels {
        let source_ports = effective_ports.entry(&resolved.source).or_default();

        if let Some(existing) = source_ports.insert(resolved.tunnel.local_port, resolved)
            && !raw_duplicate_ports.contains(&(&resolved.source, resolved.tunnel.local_port))
        {
            let (culprit, conflicting_name) = if resolved.override_source.is_some() {
                (resolved, existing.tunnel.name.as_str())
            } else if existing.override_source.is_some() {
                (existing, resolved.tunnel.name.as_str())
            } else {
                (resolved, existing.tunnel.name.as_str())
            };
            report.push(ValidationError::new(
                culprit
                    .override_source
                    .clone()
                    .unwrap_or_else(|| culprit.source.clone()),
                Some(culprit.tunnel.name.clone()),
                format!(
                    "local_port {} duplicates {}",
                    culprit.tunnel.local_port, conflicting_name
                ),
            ));
        }
    }
}

/// 権限が必要になる可能性があるローカルポートを警告する
fn warn_privileged_local_ports(config: &EffectiveConfig, report: &mut ValidationReport) {
    for (source, tunnel) in source_tunnels(config) {
        if (1..1024).contains(&tunnel.local_port) {
            report.push_warning(ValidationWarning::new(
                source.clone(),
                Some(tunnel.name.clone()),
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

    /// 同一 name のグローバル設定とローカル設定が共存することを検証する
    #[test]
    fn global_and_local_configs_keep_same_name_tunnels() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let global_path = temp_dir.path().join("global.toml");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &global_path,
            r#"
[[tunnels]]
name = "db"
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
name = "db"
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

        assert_eq!(config.tunnels.len(), 2);
        assert_eq!(config.tunnels[0].source.kind, ConfigSourceKind::Global);
        assert_eq!(config.tunnels[0].tunnel.local_port, 15432);
        assert_eq!(config.tunnels[1].source.kind, ConfigSourceKind::Local);
        assert_eq!(config.tunnels[1].tunnel.local_port, 25432);
    }

    /// local_host 未指定時に既定値が使われることを検証する
    #[test]
    fn tunnel_config_uses_default_local_host_when_omitted() {
        let tunnel = tunnel("db", 15432);

        assert_eq!(tunnel.effective_local_host(), "127.0.0.1");
    }

    /// enabled 未指定時に有効として扱われることを検証する
    #[test]
    fn tunnel_config_defaults_enabled_to_true() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &path,
            r#"
[[tunnels]]
name = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write configuration");

        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert!(loaded.tunnels[0].enabled);
    }

    /// disabled なトンネルが統合済み設定から除外されることを検証する
    #[test]
    fn disabled_tunnels_are_excluded_from_effective_config() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        fs::write(
            &path,
            r#"
[[tunnels]]
name = "enabled-db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"

[[tunnels]]
name = "disabled-db"
enabled = false
local_port = 25432
remote_host = "disabled-db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write configuration");

        let config = load_effective_config(&ConfigPaths::new(None, path)).expect("load config");

        assert_eq!(config.sources[0].tunnels.len(), 2);
        assert!(!config.sources[0].tunnels[1].enabled);
        assert_eq!(config.tunnels.len(), 1);
        assert_eq!(config.tunnels[0].tunnel.name, "enabled-db");
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
name = "db"
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
name = "db"
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
name = "db"
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
name = "db"
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

    /// 同一設定ファイル内の name 重複が検証エラーになることを検証する
    #[test]
    fn validation_reports_duplicate_names_in_same_file() {
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
                .any(|error| error.message == "Duplicate name in the same configuration file")
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

    /// disabled なトンネルも local_port 重複の検証対象になることを検証する
    #[test]
    fn validation_reports_disabled_duplicate_local_ports() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let mut disabled = tunnel("disabled-db", 15432);
        disabled.enabled = false;
        let config = EffectiveConfig::new(
            vec![LoadedConfigFile::new(
                source.clone(),
                vec![tunnel("db", 15432), disabled],
            )],
            vec![ResolvedTunnelConfig::new(source, tunnel("db", 15432))],
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

    /// global と local の同名・同ポートは設定検証エラーにしないことを検証する
    #[test]
    fn validation_allows_same_name_and_local_port_across_sources() {
        let global_source =
            ConfigSource::new(ConfigSourceKind::Global, PathBuf::from("global.toml"));
        let local_source =
            ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let config = EffectiveConfig::new(
            vec![
                LoadedConfigFile::new(global_source.clone(), vec![tunnel("db", 15432)]),
                LoadedConfigFile::new(local_source.clone(), vec![tunnel("db", 15432)]),
            ],
            vec![
                ResolvedTunnelConfig::new(global_source, tunnel("db", 15432)),
                ResolvedTunnelConfig::new(local_source, tunnel("db", 15432)),
            ],
        );

        let report = validate_config(&config);

        assert!(report.is_valid());
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

    /// disabled なトンネルも意味検証の対象になることを検証する
    #[test]
    fn validation_reports_disabled_tunnel_field_errors() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let mut invalid_tunnel = tunnel("disabled-db", 15432);
        invalid_tunnel.enabled = false;
        invalid_tunnel.tags = vec!["Invalid Tag".to_owned()];
        invalid_tunnel.local_host = Some("127.0.0.1 ".to_owned());
        invalid_tunnel.remote_port = 0;
        let config = EffectiveConfig::new(
            vec![LoadedConfigFile::new(source, vec![invalid_tunnel])],
            Vec::new(),
        );

        let report = validate_config(&config);

        assert!(!report.is_valid());
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message == "local_host cannot contain whitespace")
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message.contains("tag must contain only"))
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message == "remote_port must be greater than or equal to 1")
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
name = "db"
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

    /// 有効設定の既定値は省略し、無効設定だけ TOML に出力することを検証する
    #[test]
    fn enabled_true_is_omitted_and_false_is_serialized() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        let mut disabled = tunnel("disabled-db", 25432);
        disabled.enabled = false;

        add_tunnels_to_config_file(
            &path,
            ConfigSourceKind::Local,
            vec![tunnel("enabled-db", 15432), disabled],
        )
        .expect("add tunnels");
        let content = fs::read_to_string(path).expect("read configuration");

        assert!(!content.contains("enabled = true"));
        assert!(content.contains("enabled = false"));
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
        assert_eq!(matched[0].tunnel.name, "dev-db");
    }

    /// 除外タグ指定が OR 条件でトンネルを除外することを検証する
    #[test]
    fn filter_tunnels_excluding_tags_removes_any_matching_tag() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let mut dev_db = tunnel("dev-db", 15432);
        dev_db.tags = vec!["dev".to_owned(), "project-a".to_owned()];
        let mut prod_db = tunnel("prod-db", 25432);
        prod_db.tags = vec!["prod".to_owned(), "project-a".to_owned()];
        let mut archived_db = tunnel("archived-db", 35432);
        archived_db.tags = vec!["archived".to_owned()];
        let tunnels = vec![
            ResolvedTunnelConfig::new(source.clone(), dev_db),
            ResolvedTunnelConfig::new(source.clone(), prod_db),
            ResolvedTunnelConfig::new(source, archived_db),
        ];

        let matched =
            filter_tunnels_excluding_tags(&tunnels, &["prod".to_owned(), "archived".to_owned()]);

        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].tunnel.name, "dev-db");
    }

    /// 未正規化のトンネルタグでも除外タグ指定に一致することを検証する
    #[test]
    fn filter_tunnels_excluding_tags_compares_without_requiring_normalized_tunnel_tags() {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let mut tunnel = tunnel("prod-db", 25432);
        tunnel.tags = vec![" Prod ".to_owned(), "PROJECT-A".to_owned()];
        let tunnels = vec![ResolvedTunnelConfig::new(source, tunnel)];

        let matched = filter_tunnels_excluding_tags(&tunnels, &["prod".to_owned()]);

        assert!(matched.is_empty());
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
        let content = fs::read_to_string(&path).expect("read configuration content");

        assert_eq!(loaded.tunnels.len(), 1);
        assert_eq!(loaded.tunnels[0].name, "db");
        assert_eq!(
            loaded.timeouts.connect_timeout_seconds,
            Some(DEFAULT_CONNECT_TIMEOUT_SECONDS)
        );
        assert_eq!(
            loaded.timeouts.server_alive_interval_seconds,
            Some(DEFAULT_SERVER_ALIVE_INTERVAL_SECONDS)
        );
        assert_eq!(
            loaded.timeouts.server_alive_count_max,
            Some(DEFAULT_SERVER_ALIVE_COUNT_MAX)
        );
        assert_eq!(
            loaded.timeouts.start_grace_milliseconds,
            Some(DEFAULT_START_GRACE_MILLISECONDS)
        );
        assert!(content.contains("[timeouts]"));
        assert!(content.contains("connect_timeout_seconds = 15"));
        assert!(content.contains("server_alive_interval_seconds = 30"));
        assert!(content.contains("server_alive_count_max = 3"));
        assert!(content.contains("start_grace_milliseconds = 300"));
    }

    /// 同一設定ファイル内の name 重複が追加時に拒否されることを検証する
    #[test]
    fn add_tunnel_rejects_duplicate_id() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add tunnel");

        let result = add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 25432));

        assert!(matches!(result, Err(ConfigEditError::DuplicateName { .. })));
    }

    /// 同一設定ファイル内の local_port 重複が単体追加時に拒否されることを検証する
    #[test]
    fn add_tunnel_rejects_duplicate_local_port() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        let mut disabled = tunnel("disabled-db", 15432);
        disabled.enabled = false;
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, disabled).expect("add tunnel");

        let result = add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432));

        assert!(matches!(
            result,
            Err(ConfigEditError::DuplicateLocalPort { .. })
        ));
    }

    /// 複数トンネルを1回の書き込みで追加できることを検証する
    #[test]
    fn add_tunnels_adds_multiple_entries_atomically() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");

        add_tunnels_to_config_file(
            &path,
            ConfigSourceKind::Local,
            vec![tunnel("db", 15432), tunnel("cache", 16379)],
        )
        .expect("add tunnels");
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert_eq!(loaded.tunnels.len(), 2);
        assert_eq!(loaded.tunnels[0].name, "db");
        assert_eq!(loaded.tunnels[1].name, "cache");
    }

    /// 複数追加時の local_port 重複は保存前に拒否されることを検証する
    #[test]
    fn add_tunnels_rejects_duplicate_local_port() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add tunnel");

        let result = add_tunnels_to_config_file(
            &path,
            ConfigSourceKind::Local,
            vec![tunnel("cache", 15432)],
        );
        let loaded = read_config_file(&path, ConfigSourceKind::Local)
            .expect("read configuration")
            .expect("configuration file exists");

        assert!(matches!(
            result,
            Err(ConfigEditError::DuplicateLocalPort { .. })
        ));
        assert_eq!(loaded.tunnels.len(), 1);
    }

    /// 指定 name のトンネルが設定ファイルから削除されることを検証する
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
        assert_eq!(loaded.tunnels[0].name, "cache");
    }

    /// 指定 name のトンネルが設定ファイル内で更新されることを検証する
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
        assert_eq!(loaded.tunnels[0].name, "dev-db");
        assert_eq!(
            loaded.tunnels[0].description.as_deref(),
            Some("Development database")
        );
        assert_eq!(loaded.tunnels[1].name, "cache");
    }

    /// 更新後 name が同一設定ファイル内で重複する場合に拒否されることを検証する
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

        assert!(matches!(result, Err(ConfigEditError::DuplicateName { .. })));
    }

    /// 更新後 local_port が同一設定ファイル内で重複する場合に拒否されることを検証する
    #[test]
    fn update_tunnel_rejects_duplicate_local_port() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.toml");
        let mut disabled = tunnel("disabled-db", 16379);
        disabled.enabled = false;
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add first tunnel");
        add_tunnel_to_config_file(&path, ConfigSourceKind::Local, disabled)
            .expect("add second tunnel");

        let result = update_tunnel_in_config_file(
            &path,
            ConfigSourceKind::Local,
            "db",
            tunnel("dev-db", 16379),
        );

        assert!(matches!(
            result,
            Err(ConfigEditError::DuplicateLocalPort { .. })
        ));
    }

    /// local 設定パスから同階層の override パスが生成されることを検証する
    #[test]
    fn local_override_path_is_derived_from_local_config_path() {
        assert_eq!(
            local_override_config_path(Path::new("config/fwd-deck.toml")),
            PathBuf::from("config/fwd-deck.override.toml")
        );
        assert_eq!(
            local_override_config_path(Path::new("config/work")),
            PathBuf::from("config/work.override.toml")
        );
    }

    /// local override が同名の local トンネルだけへ部分適用されることを検証する
    #[test]
    fn local_override_applies_only_to_matching_local_tunnel() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let global_path = temp_dir.path().join("global.toml");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let paths = ConfigPaths::new(Some(global_path.clone()), local_path.clone());
        fs::write(
            &global_path,
            r#"
[[tunnels]]
name = "db"
description = "Global database"
tags = ["global"]
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
name = "db"
description = "Shared database"
tags = ["shared"]
local_port = 25432
remote_host = "local-db.internal"
remote_port = 5432
ssh_user = "local-user"
ssh_host = "local-bastion.example.com"

[tunnels.timeouts]
connect_timeout_seconds = 10
server_alive_interval_seconds = 20
"#,
        )
        .expect("write local configuration");
        fs::write(
            &paths.local_override,
            r#"
[[tunnels]]
name = "db"
enabled = false
description = "My database"
tags = []
local_port = 35432
remote_host = "override-db.internal"

[tunnels.timeouts]
connect_timeout_seconds = 5
"#,
        )
        .expect("write local override");

        let config = load_effective_config(&paths).expect("load configuration");
        let global = config
            .configured_tunnels
            .iter()
            .find(|resolved| resolved.source.kind == ConfigSourceKind::Global)
            .expect("global tunnel exists");
        let local = config
            .configured_tunnels
            .iter()
            .find(|resolved| resolved.source.kind == ConfigSourceKind::Local)
            .expect("local tunnel exists");

        assert_eq!(
            global.tunnel.description.as_deref(),
            Some("Global database")
        );
        assert!(!global.is_overridden());
        assert!(!local.tunnel.enabled);
        assert_eq!(local.tunnel.description.as_deref(), Some("My database"));
        assert!(local.tunnel.tags.is_empty());
        assert_eq!(local.tunnel.local_port, 35432);
        assert_eq!(local.tunnel.remote_port, 5432);
        assert_eq!(local.timeouts.connect_timeout_seconds, 5);
        assert_eq!(local.timeouts.server_alive_interval_seconds, 20);
        assert!(local.is_overridden());
        assert_eq!(local.source.path, local_path);
        assert_eq!(
            local
                .override_source
                .as_ref()
                .map(|source| source.path.as_path()),
            Some(paths.local_override.as_path())
        );
        assert_eq!(config.tunnels.len(), 1);
    }

    /// 本体に存在しない override が警告されて無視されることを検証する
    #[test]
    fn unknown_local_override_is_warned_and_ignored() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let paths = ConfigPaths::new(None, local_path.clone());
        fs::write(
            &local_path,
            r#"
[[tunnels]]
name = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write local configuration");
        fs::write(
            &paths.local_override,
            r#"
[[tunnels]]
name = "missing"
enabled = false
"#,
        )
        .expect("write local override");

        let config = load_effective_config(&paths).expect("load configuration");
        let report = validate_config(&config);

        assert!(report.is_valid());
        assert_eq!(config.tunnels.len(), 1);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].tunnel_name.as_deref(), Some("missing"));
        assert_eq!(report.warnings[0].source.path, paths.local_override);
    }

    /// local override 内の name 重複が検証エラーになることを検証する
    #[test]
    fn duplicate_local_override_name_is_rejected_by_validation() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let paths = ConfigPaths::new(None, local_path.clone());
        fs::write(
            &local_path,
            r#"
[[tunnels]]
name = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write local configuration");
        fs::write(
            &paths.local_override,
            r#"
[[tunnels]]
name = "db"
enabled = false

[[tunnels]]
name = "db"
description = "My database"
"#,
        )
        .expect("write local override");

        let config = load_effective_config(&paths).expect("load configuration");
        let report = validate_config(&config);

        assert!(!report.is_valid());
        assert!(report.errors.iter().any(|error| {
            error.tunnel_name.as_deref() == Some("db")
                && error.message == "Duplicate name in local override configuration file"
        }));
    }

    /// local override が作る実効 local_port 重複が override 側のエラーになることを検証する
    #[test]
    fn local_override_duplicate_effective_port_is_rejected_by_validation() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let paths = ConfigPaths::new(None, local_path.clone());
        fs::write(
            &local_path,
            r#"
[[tunnels]]
name = "db"
local_port = 15432
remote_host = "db.internal"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"

[[tunnels]]
name = "cache"
local_port = 16379
remote_host = "cache.internal"
remote_port = 6379
ssh_user = "user"
ssh_host = "bastion.example.com"
"#,
        )
        .expect("write local configuration");
        fs::write(
            &paths.local_override,
            r#"
[[tunnels]]
name = "db"
local_port = 16379
"#,
        )
        .expect("write local override");

        let config = load_effective_config(&paths).expect("load configuration");
        let report = validate_config(&config);

        assert!(!report.is_valid());
        assert!(report.errors.iter().any(|error| {
            error.source.path == paths.local_override
                && error.tunnel_name.as_deref() == Some("db")
                && error.message == "local_port 16379 duplicates cache"
        }));
    }

    /// local override 内の未知フィールドが読み込みエラーになることを検証する
    #[test]
    fn unknown_local_override_field_is_rejected_during_load() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let paths = ConfigPaths::new(None, local_path.clone());
        fs::write(&local_path, "").expect("write local configuration");
        fs::write(
            &paths.local_override,
            r#"
[[tunnels]]
name = "db"
unknown = true
"#,
        )
        .expect("write local override");

        let result = load_effective_config(&paths);

        assert!(matches!(result, Err(ConfigLoadError::Parse { .. })));
    }

    /// override の作成、更新、最終解除でファイルが削除されることを検証する
    #[test]
    fn tunnel_override_upsert_and_remove_manage_override_file() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("fwd-deck.override.toml");
        let mut tunnel_override = TunnelConfigOverride::new("db".to_owned());
        tunnel_override.description = Some("My database".to_owned());

        upsert_tunnel_override_in_config_file(&path, tunnel_override)
            .expect("upsert tunnel override");
        let loaded = read_config_override_file(&path)
            .expect("read tunnel override")
            .expect("override file exists");
        assert_eq!(
            loaded.tunnels[0].description.as_deref(),
            Some("My database")
        );

        remove_tunnel_override_from_config_file(&path, "db").expect("remove tunnel override");
        assert!(!path.exists());
    }

    /// 本体 rename と削除に override が追従することを検証する
    #[test]
    fn local_config_rename_and_remove_keep_override_in_sync() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let override_path = temp_dir.path().join("fwd-deck.override.toml");
        add_tunnel_to_config_file(&local_path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add local tunnel");
        let mut tunnel_override = TunnelConfigOverride::new("db".to_owned());
        tunnel_override.enabled = Some(false);
        upsert_tunnel_override_in_config_file(&override_path, tunnel_override)
            .expect("add tunnel override");

        update_tunnel_and_override_name_in_config_files(
            &local_path,
            &override_path,
            ConfigSourceKind::Local,
            "db",
            tunnel("dev-db", 15432),
        )
        .expect("rename local tunnel");
        let renamed = read_config_override_file(&override_path)
            .expect("read renamed override")
            .expect("override file exists");
        assert_eq!(renamed.tunnels[0].name, "dev-db");

        remove_tunnel_and_override_from_config_files(
            &local_path,
            &override_path,
            ConfigSourceKind::Local,
            "dev-db",
        )
        .expect("remove local tunnel");
        assert!(!override_path.exists());
    }

    /// override の name 重複時に本体 rename が先行しないことを検証する
    #[test]
    fn local_config_rename_preflights_duplicate_override_name() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let local_path = temp_dir.path().join("fwd-deck.toml");
        let override_path = temp_dir.path().join("fwd-deck.override.toml");
        add_tunnel_to_config_file(&local_path, ConfigSourceKind::Local, tunnel("db", 15432))
            .expect("add local tunnel");
        fs::write(
            &override_path,
            r#"
[[tunnels]]
name = "db"
enabled = false

[[tunnels]]
name = "db"
description = "My database"
"#,
        )
        .expect("write duplicate override");

        let result = update_tunnel_and_override_name_in_config_files(
            &local_path,
            &override_path,
            ConfigSourceKind::Local,
            "db",
            tunnel("dev-db", 15432),
        );

        assert!(matches!(result, Err(ConfigEditError::DuplicateName { .. })));
        let local = read_config_file(&local_path, ConfigSourceKind::Local)
            .expect("read local configuration")
            .expect("local configuration exists");
        assert_eq!(local.tunnels[0].name, "db");
    }

    /// テスト用のトンネル設定を生成する
    fn tunnel(name: &str, local_port: u16) -> TunnelConfig {
        TunnelConfig {
            name: name.to_owned(),
            enabled: true,
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
