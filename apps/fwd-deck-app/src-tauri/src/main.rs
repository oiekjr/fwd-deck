use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use fwd_deck_core::{
    ConfigEditError, ConfigLoadError, ConfigPaths, ConfigSourceKind, EffectiveConfig, ProcessState,
    ResolvedTimeoutConfig, ResolvedTunnelConfig, StartedTunnel, StoppedTunnel, TimeoutConfig,
    TunnelConfig, TunnelRuntimeError, TunnelRuntimeStatus, ValidationReport,
    add_tunnel_to_config_file, default_global_config_path, default_local_config_path,
    default_state_file_path, load_effective_config, remove_tunnel_from_config_file, start_tunnel,
    stop_tunnel, tag_is_valid, tunnel_statuses, validate_config,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tauri アプリを起動する
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_dashboard,
            start_tunnels,
            stop_tunnels,
            add_tunnel_entry,
            remove_tunnel_entry
        ])
        .run(tauri::generate_context!())
        .expect("error while running fwd-deck application");
}

/// フロントエンドから指定する設定ファイルと状態ファイルのパスを表現する
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathSelection {
    local_config_path: Option<String>,
    global_config_path: Option<String>,
    use_global: bool,
    state_path: Option<String>,
}

impl Default for PathSelection {
    /// 既定のパス選択を初期化する
    fn default() -> Self {
        Self {
            local_config_path: None,
            global_config_path: None,
            use_global: true,
            state_path: None,
        }
    }
}

/// 解決済みの実行時パスを表現する
#[derive(Debug, Clone)]
struct RuntimePaths {
    config_paths: ConfigPaths,
    state_path: PathBuf,
}

/// フロントエンドへ返す解決済みパスを表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PathView {
    local_config_path: String,
    global_config_path: Option<String>,
    use_global: bool,
    state_path: String,
}

/// ダッシュボード表示に必要な状態を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardState {
    paths: PathView,
    has_config: bool,
    validation: ValidationView,
    tunnels: Vec<TunnelView>,
    tracked_tunnels: Vec<TrackedTunnelView>,
}

/// 設定検証結果の表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationView {
    is_valid: bool,
    errors: Vec<ValidationIssueView>,
    warnings: Vec<ValidationIssueView>,
}

/// 設定検証で検出した問題の表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationIssueView {
    source: String,
    path: String,
    tunnel_id: Option<String>,
    message: String,
}

/// 設定済みトンネルの表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelView {
    id: String,
    description: Option<String>,
    tags: Vec<String>,
    local: String,
    remote: String,
    ssh: String,
    source: String,
    source_path: String,
    timeouts: TimeoutView,
    status: Option<RuntimeStatusView>,
}

/// 起動中または stale なトンネル状態の表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackedTunnelView {
    id: String,
    local: String,
    remote: String,
    ssh: String,
    status: RuntimeStatusView,
}

/// runtime 状態の表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatusView {
    pid: u32,
    state: String,
    source: String,
    source_path: String,
    started_at_unix_seconds: u64,
}

/// タイムアウト設定の表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimeoutView {
    connect_timeout_seconds: u32,
    server_alive_interval_seconds: u32,
    server_alive_count_max: u32,
    start_grace_milliseconds: u64,
}

/// トンネル操作結果を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationReport {
    succeeded: Vec<OperationSuccessView>,
    failed: Vec<OperationFailureView>,
}

/// 成功したトンネル操作を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationSuccessView {
    id: String,
    message: String,
}

/// 失敗したトンネル操作を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationFailureView {
    id: String,
    message: String,
}

/// 設定編集対象のスコープを表現する
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ConfigScopeInput {
    Global,
    Local,
}

impl From<ConfigScopeInput> for ConfigSourceKind {
    /// フロントエンド入力を設定ファイル種別へ変換する
    fn from(scope: ConfigScopeInput) -> Self {
        match scope {
            ConfigScopeInput::Global => Self::Global,
            ConfigScopeInput::Local => Self::Local,
        }
    }
}

/// 設定追加フォームから受け取るトンネル入力を表現する
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TunnelInput {
    id: String,
    description: Option<String>,
    tags: Vec<String>,
    local_host: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
    ssh_user: String,
    ssh_host: String,
    ssh_port: Option<u16>,
    identity_file: Option<String>,
}

impl TunnelInput {
    /// 入力値を中核機能のトンネル設定へ変換する
    fn into_tunnel_config(self) -> TunnelConfig {
        TunnelConfig {
            id: trimmed_required(self.id),
            description: trimmed_optional(self.description),
            tags: self.tags.into_iter().map(trimmed_required).collect(),
            local_host: Some(trimmed_required(self.local_host)),
            local_port: self.local_port,
            remote_host: trimmed_required(self.remote_host),
            remote_port: self.remote_port,
            ssh_user: trimmed_required(self.ssh_user),
            ssh_host: trimmed_required(self.ssh_host),
            ssh_port: self.ssh_port,
            identity_file: trimmed_optional(self.identity_file),
            timeouts: TimeoutConfig::default(),
        }
    }
}

/// Tauri command の失敗理由を表現する
#[derive(Debug, Error)]
enum AppError {
    #[error("現在のディレクトリを取得できませんでした: {0}")]
    CurrentDir(std::io::Error),
    #[error("HOME が未設定のため、グローバル設定ファイルの既定パスを解決できません")]
    MissingGlobalConfigPath,
    #[error("HOME が未設定のため、状態ファイルの既定パスを解決できません")]
    MissingStatePath,
    #[error("入力が不正です: {0}")]
    InvalidInput(String),
    #[error("設定にエラーがあります: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    ConfigLoad(#[from] ConfigLoadError),
    #[error(transparent)]
    ConfigEdit(#[from] ConfigEditError),
    #[error(transparent)]
    Runtime(#[from] TunnelRuntimeError),
}

/// ダッシュボード表示に必要な情報を取得する
#[tauri::command]
fn load_dashboard(paths: Option<PathSelection>) -> Result<DashboardState, String> {
    command_result(load_dashboard_inner(paths))
}

/// 指定トンネルを開始する
#[tauri::command]
fn start_tunnels(
    paths: Option<PathSelection>,
    ids: Vec<String>,
) -> Result<OperationReport, String> {
    command_result(start_tunnels_inner(paths, ids))
}

/// 指定トンネルを停止する
#[tauri::command]
fn stop_tunnels(paths: Option<PathSelection>, ids: Vec<String>) -> Result<OperationReport, String> {
    command_result(stop_tunnels_inner(paths, ids))
}

/// 設定ファイルへトンネルを追加する
#[tauri::command]
fn add_tunnel_entry(
    paths: Option<PathSelection>,
    scope: ConfigScopeInput,
    tunnel: TunnelInput,
) -> Result<DashboardState, String> {
    command_result(add_tunnel_entry_inner(paths, scope, tunnel))
}

/// 設定ファイルからトンネルを削除する
#[tauri::command]
fn remove_tunnel_entry(
    paths: Option<PathSelection>,
    scope: ConfigScopeInput,
    id: String,
) -> Result<DashboardState, String> {
    command_result(remove_tunnel_entry_inner(paths, scope, &id))
}

/// command の内部エラーをフロントエンド用文字列へ変換する
fn command_result<T>(result: Result<T, AppError>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

/// ダッシュボード状態を組み立てる
fn load_dashboard_inner(paths: Option<PathSelection>) -> Result<DashboardState, AppError> {
    let runtime_paths = resolve_runtime_paths(paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let statuses = tunnel_statuses(&runtime_paths.state_path)?;
    let validation = validate_config(&config);

    Ok(build_dashboard_state(
        runtime_paths,
        config,
        statuses,
        validation,
    ))
}

/// トンネル開始処理を実行する
fn start_tunnels_inner(
    paths: Option<PathSelection>,
    ids: Vec<String>,
) -> Result<OperationReport, AppError> {
    let runtime_paths = resolve_runtime_paths(paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;

    ensure_valid_config(&config)?;
    run_tunnel_operations(&ids, |id| {
        let tunnel = find_tunnel_by_id(&config, id)
            .ok_or_else(|| AppError::InvalidInput(format!("未定義のトンネル ID です: {id}")))?;
        start_tunnel(tunnel, &runtime_paths.state_path)
            .map(start_success_message)
            .map_err(AppError::Runtime)
    })
}

/// トンネル停止処理を実行する
fn stop_tunnels_inner(
    paths: Option<PathSelection>,
    ids: Vec<String>,
) -> Result<OperationReport, AppError> {
    let runtime_paths = resolve_runtime_paths(paths)?;

    run_tunnel_operations(&ids, |id| {
        stop_tunnel(id, &runtime_paths.state_path)
            .map(stop_success_message)
            .map_err(AppError::Runtime)
    })
}

/// トンネル追加処理を実行する
fn add_tunnel_entry_inner(
    paths: Option<PathSelection>,
    scope: ConfigScopeInput,
    tunnel: TunnelInput,
) -> Result<DashboardState, AppError> {
    let runtime_paths = resolve_runtime_paths(paths.clone())?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let tunnel = tunnel.into_tunnel_config();

    validate_new_tunnel(&config, &tunnel)?;

    let kind = ConfigSourceKind::from(scope);
    let path = config_path_for_scope(&runtime_paths.config_paths, kind)?;
    add_tunnel_to_config_file(&path, kind, tunnel)?;
    load_dashboard_inner(paths)
}

/// トンネル削除処理を実行する
fn remove_tunnel_entry_inner(
    paths: Option<PathSelection>,
    scope: ConfigScopeInput,
    id: &str,
) -> Result<DashboardState, AppError> {
    let runtime_paths = resolve_runtime_paths(paths.clone())?;
    let kind = ConfigSourceKind::from(scope);
    let path = config_path_for_scope(&runtime_paths.config_paths, kind)?;

    remove_tunnel_from_config_file(&path, kind, id)?;
    load_dashboard_inner(paths)
}

/// CLI と同じ既定値に基づいて実行時パスを解決する
fn resolve_runtime_paths(paths: Option<PathSelection>) -> Result<RuntimePaths, AppError> {
    let current_dir = env::current_dir().map_err(AppError::CurrentDir)?;
    let selection = paths.unwrap_or_default();
    let local = non_empty_path(selection.local_config_path.clone())
        .unwrap_or_else(|| default_local_config_path(&current_dir));
    let global = resolve_global_config_path(&selection)?;
    let state_path = non_empty_path(selection.state_path)
        .or_else(default_state_file_path)
        .ok_or(AppError::MissingStatePath)?;

    Ok(RuntimePaths {
        config_paths: ConfigPaths::new(global, local),
        state_path,
    })
}

/// グローバル設定ファイルのパスを解決する
fn resolve_global_config_path(selection: &PathSelection) -> Result<Option<PathBuf>, AppError> {
    if !selection.use_global {
        return Ok(None);
    }

    non_empty_path(selection.global_config_path.clone())
        .or_else(default_global_config_path)
        .map(Some)
        .ok_or(AppError::MissingGlobalConfigPath)
}

/// 空文字列を除外して PathBuf へ変換する
fn non_empty_path(path: Option<String>) -> Option<PathBuf> {
    path.map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// 対象スコープの設定ファイルパスを取得する
fn config_path_for_scope(paths: &ConfigPaths, kind: ConfigSourceKind) -> Result<PathBuf, AppError> {
    match kind {
        ConfigSourceKind::Global => paths
            .global
            .clone()
            .ok_or(AppError::MissingGlobalConfigPath),
        ConfigSourceKind::Local => Ok(paths.local.clone()),
    }
}

/// ダッシュボード状態へ変換する
fn build_dashboard_state(
    runtime_paths: RuntimePaths,
    config: EffectiveConfig,
    statuses: Vec<TunnelRuntimeStatus>,
    validation: ValidationReport,
) -> DashboardState {
    let status_by_id = statuses
        .iter()
        .map(|status| (status.state.id.as_str(), status))
        .collect::<HashMap<_, _>>();
    let mut tunnels = config
        .tunnels
        .iter()
        .map(|resolved| {
            tunnel_view(
                resolved,
                status_by_id.get(resolved.tunnel.id.as_str()).copied(),
            )
        })
        .collect::<Vec<_>>();
    let mut tracked_tunnels = statuses.iter().map(tracked_tunnel_view).collect::<Vec<_>>();

    tunnels.sort_by(|left, right| left.id.cmp(&right.id));
    tracked_tunnels.sort_by(|left, right| left.id.cmp(&right.id));

    DashboardState {
        paths: path_view(&runtime_paths),
        has_config: config.has_sources(),
        validation: validation_view(validation),
        tunnels,
        tracked_tunnels,
    }
}

/// 解決済みパスを表示用へ変換する
fn path_view(paths: &RuntimePaths) -> PathView {
    PathView {
        local_config_path: display_path(&paths.config_paths.local),
        global_config_path: paths.config_paths.global.as_deref().map(display_path),
        use_global: paths.config_paths.global.is_some(),
        state_path: display_path(&paths.state_path),
    }
}

/// 設定検証結果を表示用へ変換する
fn validation_view(report: ValidationReport) -> ValidationView {
    ValidationView {
        is_valid: report.is_valid(),
        errors: report
            .errors
            .into_iter()
            .map(|issue| ValidationIssueView {
                source: issue.source.kind.to_string(),
                path: display_path(&issue.source.path),
                tunnel_id: issue.tunnel_id,
                message: issue.message,
            })
            .collect(),
        warnings: report
            .warnings
            .into_iter()
            .map(|issue| ValidationIssueView {
                source: issue.source.kind.to_string(),
                path: display_path(&issue.source.path),
                tunnel_id: issue.tunnel_id,
                message: issue.message,
            })
            .collect(),
    }
}

/// 設定済みトンネルを表示用へ変換する
fn tunnel_view(
    resolved: &ResolvedTunnelConfig,
    status: Option<&TunnelRuntimeStatus>,
) -> TunnelView {
    let tunnel = &resolved.tunnel;

    TunnelView {
        id: tunnel.id.clone(),
        description: tunnel.description.clone(),
        tags: tunnel.tags.clone(),
        local: format_local_endpoint(tunnel),
        remote: format_remote_endpoint(tunnel),
        ssh: format_ssh_endpoint(tunnel),
        source: resolved.source.kind.to_string(),
        source_path: display_path(&resolved.source.path),
        timeouts: timeout_view(resolved.timeouts),
        status: status.map(runtime_status_view),
    }
}

/// 追跡中トンネルを表示用へ変換する
fn tracked_tunnel_view(status: &TunnelRuntimeStatus) -> TrackedTunnelView {
    TrackedTunnelView {
        id: status.state.id.clone(),
        local: format_state_local_endpoint(status),
        remote: format_state_remote_endpoint(status),
        ssh: format_state_ssh_endpoint(status),
        status: runtime_status_view(status),
    }
}

/// runtime 状態を表示用へ変換する
fn runtime_status_view(status: &TunnelRuntimeStatus) -> RuntimeStatusView {
    RuntimeStatusView {
        pid: status.state.pid,
        state: match status.process_state {
            ProcessState::Running => "running".to_owned(),
            ProcessState::Stale => "stale".to_owned(),
        },
        source: status.state.source_kind.to_string(),
        source_path: display_path(&status.state.source_path),
        started_at_unix_seconds: status.state.started_at_unix_seconds,
    }
}

/// タイムアウト設定を表示用へ変換する
fn timeout_view(timeouts: ResolvedTimeoutConfig) -> TimeoutView {
    TimeoutView {
        connect_timeout_seconds: timeouts.connect_timeout_seconds,
        server_alive_interval_seconds: timeouts.server_alive_interval_seconds,
        server_alive_count_max: timeouts.server_alive_count_max,
        start_grace_milliseconds: timeouts.start_grace_milliseconds,
    }
}

/// 設定が開始可能な状態であることを検証する
fn ensure_valid_config(config: &EffectiveConfig) -> Result<(), AppError> {
    let report = validate_config(config);

    if report.is_valid() {
        return Ok(());
    }

    Err(AppError::InvalidConfig(
        report
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join(", "),
    ))
}

/// 追加対象トンネルの意味的な不備を検証する
fn validate_new_tunnel(config: &EffectiveConfig, tunnel: &TunnelConfig) -> Result<(), AppError> {
    ensure_required("id", &tunnel.id)?;
    ensure_required("local_host", tunnel.effective_local_host())?;
    ensure_required("remote_host", &tunnel.remote_host)?;
    ensure_required("ssh_user", &tunnel.ssh_user)?;
    ensure_required("ssh_host", &tunnel.ssh_host)?;

    if tunnel
        .effective_local_host()
        .chars()
        .any(char::is_whitespace)
    {
        return Err(AppError::InvalidInput(
            "local_host に空白文字は使用できません".to_owned(),
        ));
    }

    for tag in &tunnel.tags {
        if !tag_is_valid(tag) {
            return Err(AppError::InvalidInput(format!(
                "tag は小文字 ASCII、数字、'-'、'_'、'.'、'/' のみ使用できます: {tag}"
            )));
        }
    }

    if let Some(existing) = config
        .tunnels
        .iter()
        .find(|resolved| resolved.tunnel.id == tunnel.id)
    {
        return Err(AppError::InvalidInput(format!(
            "同じ ID のトンネルが既に存在します: {} ({})",
            existing.tunnel.id,
            display_path(&existing.source.path)
        )));
    }

    if let Some(existing) = config
        .tunnels
        .iter()
        .find(|resolved| resolved.tunnel.local_port == tunnel.local_port)
    {
        return Err(AppError::InvalidInput(format!(
            "local_port は既存トンネルと重複しています: {} ({})",
            tunnel.local_port, existing.tunnel.id
        )));
    }

    Ok(())
}

/// 必須入力が空でないことを検証する
fn ensure_required(name: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::InvalidInput(format!("{name} は必須です")));
    }

    Ok(())
}

/// 複数トンネルに対する操作を順次実行する
fn run_tunnel_operations<F>(ids: &[String], mut operation: F) -> Result<OperationReport, AppError>
where
    F: FnMut(&str) -> Result<String, AppError>,
{
    if ids.is_empty() {
        return Err(AppError::InvalidInput(
            "操作対象のトンネルが選択されていません".to_owned(),
        ));
    }

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for id in ids {
        match operation(id) {
            Ok(message) => succeeded.push(OperationSuccessView {
                id: id.clone(),
                message,
            }),
            Err(error) => failed.push(OperationFailureView {
                id: id.clone(),
                message: error.to_string(),
            }),
        }
    }

    Ok(OperationReport { succeeded, failed })
}

/// 開始成功時のメッセージを生成する
fn start_success_message(started: StartedTunnel) -> String {
    format!(
        "{} を開始しました (pid: {})",
        started.state.id, started.state.pid
    )
}

/// 停止成功時のメッセージを生成する
fn stop_success_message(stopped: StoppedTunnel) -> String {
    match stopped.previous_state {
        ProcessState::Running => format!(
            "{} を停止しました (pid: {})",
            stopped.state.id, stopped.state.pid
        ),
        ProcessState::Stale => format!(
            "{} の stale 状態を削除しました (pid: {})",
            stopped.state.id, stopped.state.pid
        ),
    }
}

/// 指定 ID のトンネル設定を取得する
fn find_tunnel_by_id<'a>(
    config: &'a EffectiveConfig,
    id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .find(|resolved| resolved.tunnel.id == id)
}

/// トンネル設定の local endpoint を生成する
fn format_local_endpoint(tunnel: &TunnelConfig) -> String {
    format!("{}:{}", tunnel.effective_local_host(), tunnel.local_port)
}

/// トンネル設定の remote endpoint を生成する
fn format_remote_endpoint(tunnel: &TunnelConfig) -> String {
    format!("{}:{}", tunnel.remote_host, tunnel.remote_port)
}

/// トンネル設定の SSH endpoint を生成する
fn format_ssh_endpoint(tunnel: &TunnelConfig) -> String {
    match tunnel.ssh_port {
        Some(port) => format!("{}@{}:{}", tunnel.ssh_user, tunnel.ssh_host, port),
        None => format!("{}@{}", tunnel.ssh_user, tunnel.ssh_host),
    }
}

/// 状態ファイルの local endpoint を生成する
fn format_state_local_endpoint(status: &TunnelRuntimeStatus) -> String {
    format!("{}:{}", status.state.local_host, status.state.local_port)
}

/// 状態ファイルの remote endpoint を生成する
fn format_state_remote_endpoint(status: &TunnelRuntimeStatus) -> String {
    format!("{}:{}", status.state.remote_host, status.state.remote_port)
}

/// 状態ファイルの SSH endpoint を生成する
fn format_state_ssh_endpoint(status: &TunnelRuntimeStatus) -> String {
    match status.state.ssh_port {
        Some(port) => format!(
            "{}@{}:{}",
            status.state.ssh_user, status.state.ssh_host, port
        ),
        None => format!("{}@{}", status.state.ssh_user, status.state.ssh_host),
    }
}

/// Path を表示用文字列へ変換する
fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// 必須入力の前後空白を除去する
fn trimmed_required(value: String) -> String {
    value.trim().to_owned()
}

/// 任意入力の前後空白を除去し、空文字列を未指定として扱う
fn trimmed_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
