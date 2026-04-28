use std::{
    collections::HashMap,
    fs,
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
use tauri::Manager;
use thiserror::Error;

const APP_PREFERENCES_FILE_NAME: &str = "preferences.toml";
const APP_PREFERENCES_VERSION: u32 = 1;
const WORKSPACE_HISTORY_LIMIT: usize = 10;
const WORKSPACE_STATES_DIR: &str = "workspace-states";
const STATE_FILE_NAME: &str = "state.toml";

/// Tauri アプリを起動する
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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

/// フロントエンドから指定するワークスペース選択を表現する
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceSelection {
    workspace_path: Option<String>,
    global_config_path: Option<String>,
    use_global: Option<bool>,
}

/// アプリ固有の設定を表現する
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct AppPreferences {
    version: u32,
    active_workspace_path: Option<PathBuf>,
    workspace_history: Vec<PathBuf>,
    use_global: bool,
    global_config_path: Option<PathBuf>,
}

impl Default for AppPreferences {
    /// 既定のアプリ設定を初期化する
    fn default() -> Self {
        Self {
            version: APP_PREFERENCES_VERSION,
            active_workspace_path: None,
            workspace_history: Vec::new(),
            use_global: true,
            global_config_path: None,
        }
    }
}

/// 解決済みの実行時パスを表現する
#[derive(Debug, Clone)]
struct RuntimePaths {
    preferences: AppPreferences,
    config_paths: ConfigPaths,
    local_config_path: Option<PathBuf>,
    global_config_display_path: Option<PathBuf>,
    global_state_path: PathBuf,
    workspace_state_path: Option<PathBuf>,
}

/// フロントエンドへ返す解決済みパスを表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PathView {
    workspace_path: String,
    workspace_history: Vec<String>,
    local_config_path: String,
    global_config_path: String,
    use_global: bool,
    global_state_path: String,
    workspace_state_path: String,
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
    runtime_scope: RuntimeScope,
    runtime_key: String,
    local: String,
    remote: String,
    ssh: String,
    status: RuntimeStatusView,
}

/// runtime 状態の表示情報を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatusView {
    runtime_scope: RuntimeScope,
    runtime_key: String,
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

/// runtime 状態ファイルのスコープを表現する
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
enum RuntimeScope {
    Global,
    Workspace,
}

impl std::fmt::Display for RuntimeScope {
    /// runtime scope の表示用文字列を生成する
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => formatter.write_str("global"),
            Self::Workspace => formatter.write_str("workspace"),
        }
    }
}

/// トンネル操作対象を表現する
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationTargetInput {
    id: String,
    runtime_scope: Option<RuntimeScope>,
}

/// runtime scope を付与したトンネル状態を表現する
#[derive(Debug, Clone)]
struct ScopedRuntimeStatus {
    runtime_scope: RuntimeScope,
    status: TunnelRuntimeStatus,
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
    #[error("アプリ設定ディレクトリを解決できませんでした: {0}")]
    AppConfigDir(tauri::Error),
    #[error("HOME が未設定のため、グローバル設定ファイルの既定パスを解決できません")]
    MissingGlobalConfigPath,
    #[error("HOME が未設定のため、状態ファイルの既定パスを解決できません")]
    MissingStatePath,
    #[error("local 設定を操作するにはワークスペースを選択してください")]
    MissingWorkspace,
    #[error("ワークスペースディレクトリが存在しません: {path}")]
    WorkspaceNotFound { path: PathBuf },
    #[error("アプリ設定を読み込めませんでした: {path}: {source}")]
    PreferencesRead {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("アプリ設定を解析できませんでした: {path}: {source}")]
    PreferencesParse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("アプリ設定をシリアライズできませんでした: {path}: {source}")]
    PreferencesSerialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("アプリ設定ディレクトリを作成できませんでした: {path}: {source}")]
    PreferencesCreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("アプリ設定を書き込めませんでした: {path}: {source}")]
    PreferencesWrite {
        path: PathBuf,
        source: std::io::Error,
    },
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
fn load_dashboard(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
) -> Result<DashboardState, String> {
    command_result(load_dashboard_inner(&app, paths))
}

/// 指定トンネルを開始する
#[tauri::command]
fn start_tunnels(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
) -> Result<OperationReport, String> {
    command_result(start_tunnels_inner(&app, paths, targets))
}

/// 指定トンネルを停止する
#[tauri::command]
fn stop_tunnels(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
) -> Result<OperationReport, String> {
    command_result(stop_tunnels_inner(&app, paths, targets))
}

/// 設定ファイルへトンネルを追加する
#[tauri::command]
fn add_tunnel_entry(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    scope: ConfigScopeInput,
    tunnel: TunnelInput,
) -> Result<DashboardState, String> {
    command_result(add_tunnel_entry_inner(&app, paths, scope, tunnel))
}

/// 設定ファイルからトンネルを削除する
#[tauri::command]
fn remove_tunnel_entry(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    scope: ConfigScopeInput,
    id: String,
) -> Result<DashboardState, String> {
    command_result(remove_tunnel_entry_inner(&app, paths, scope, &id))
}

/// command の内部エラーをフロントエンド用文字列へ変換する
fn command_result<T>(result: Result<T, AppError>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

/// ダッシュボード状態を組み立てる
fn load_dashboard_inner(
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
) -> Result<DashboardState, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let statuses = load_scoped_runtime_statuses(&runtime_paths)?;
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
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
) -> Result<OperationReport, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let tunnels_by_id = tunnel_index_by_id(&config);

    ensure_valid_config(&config)?;
    run_tunnel_operations(&targets, |target| {
        let tunnel = tunnels_by_id
            .get(target.id.as_str())
            .copied()
            .ok_or_else(|| {
                AppError::InvalidInput(format!("未定義のトンネル ID です: {}", target.id))
            })?;
        let state_path = state_path_for_source(&runtime_paths, tunnel.source.kind)?;

        start_tunnel_for_app(tunnel, state_path)
    })
}

/// トンネル停止処理を実行する
fn stop_tunnels_inner(
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
) -> Result<OperationReport, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let tunnels_by_id = tunnel_index_by_id(&config);

    run_tunnel_operations(&targets, |target| {
        let state_path = match target.runtime_scope {
            Some(scope) => state_path_for_runtime_scope(&runtime_paths, scope)?,
            None => {
                let tunnel = tunnels_by_id
                    .get(target.id.as_str())
                    .copied()
                    .ok_or_else(|| {
                        AppError::InvalidInput(format!("未定義のトンネル ID です: {}", target.id))
                    })?;
                state_path_for_source(&runtime_paths, tunnel.source.kind)?
            }
        };

        stop_tunnel_for_app(&target.id, state_path)
    })
}

/// トンネル追加処理を実行する
fn add_tunnel_entry_inner(
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    scope: ConfigScopeInput,
    tunnel: TunnelInput,
) -> Result<DashboardState, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths.clone())?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let tunnel = tunnel.into_tunnel_config();

    validate_new_tunnel(&config, &tunnel)?;

    let kind = ConfigSourceKind::from(scope);
    let path = config_path_for_scope(&runtime_paths, kind)?;
    add_tunnel_to_config_file(&path, kind, tunnel)?;
    load_dashboard_inner(app, paths)
}

/// トンネル削除処理を実行する
fn remove_tunnel_entry_inner(
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    scope: ConfigScopeInput,
    id: &str,
) -> Result<DashboardState, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths.clone())?;
    let kind = ConfigSourceKind::from(scope);
    let path = config_path_for_scope(&runtime_paths, kind)?;

    remove_tunnel_from_config_file(&path, kind, id)?;
    load_dashboard_inner(app, paths)
}

/// アプリ設定と入力から実行時パスを解決する
fn resolve_runtime_paths(
    app: &tauri::AppHandle,
    selection: Option<WorkspaceSelection>,
) -> Result<RuntimePaths, AppError> {
    let app_config_dir = app_config_dir(app)?;
    let preferences_path = preferences_path_from_app_config_dir(&app_config_dir);
    let mut preferences = read_preferences_file(&preferences_path)?;

    normalize_loaded_preferences(&mut preferences);

    if let Some(selection) = selection {
        apply_workspace_selection(&mut preferences, selection)?;
    }

    let global = resolve_global_config_path(&preferences)?;
    let global_config_display_path = preferences
        .global_config_path
        .clone()
        .or_else(default_global_config_path);
    let local_config_path = preferences
        .active_workspace_path
        .as_deref()
        .map(default_local_config_path);
    let config_local_path = local_config_path
        .clone()
        .unwrap_or_else(|| no_workspace_local_config_path(&app_config_dir));
    let global_state_path = default_state_file_path().ok_or(AppError::MissingStatePath)?;
    let workspace_state_path = preferences
        .active_workspace_path
        .as_deref()
        .map(|workspace| workspace_state_file_path(&app_config_dir, workspace));

    write_preferences_file(&preferences_path, &preferences)?;

    Ok(RuntimePaths {
        preferences,
        config_paths: ConfigPaths::new(global, config_local_path),
        local_config_path,
        global_config_display_path,
        global_state_path,
        workspace_state_path,
    })
}

/// Tauri の app config directory を取得する
fn app_config_dir(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    app.path().app_config_dir().map_err(AppError::AppConfigDir)
}

/// アプリ設定ファイルのパスを生成する
fn preferences_path_from_app_config_dir(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join(APP_PREFERENCES_FILE_NAME)
}

/// アプリ設定ファイルを読み込む
fn read_preferences_file(path: &Path) -> Result<AppPreferences, AppError> {
    if !path.exists() {
        return Ok(AppPreferences::default());
    }

    let content = fs::read_to_string(path).map_err(|source| AppError::PreferencesRead {
        path: path.to_path_buf(),
        source,
    })?;

    toml::from_str::<AppPreferences>(&content).map_err(|source| AppError::PreferencesParse {
        path: path.to_path_buf(),
        source,
    })
}

/// アプリ設定ファイルを書き込む
fn write_preferences_file(path: &Path, preferences: &AppPreferences) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| AppError::PreferencesCreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let content =
        toml::to_string_pretty(preferences).map_err(|source| AppError::PreferencesSerialize {
            path: path.to_path_buf(),
            source,
        })?;

    fs::write(path, content).map_err(|source| AppError::PreferencesWrite {
        path: path.to_path_buf(),
        source,
    })
}

/// 保存済み設定の実行不能な値を取り除く
fn normalize_loaded_preferences(preferences: &mut AppPreferences) {
    preferences.version = APP_PREFERENCES_VERSION;
    preferences.active_workspace_path = preferences
        .active_workspace_path
        .as_deref()
        .and_then(canonical_workspace_path_if_available);
    preferences.workspace_history = preferences
        .workspace_history
        .iter()
        .filter_map(|path| canonical_workspace_path_if_available(path))
        .fold(Vec::<PathBuf>::new(), |mut history, path| {
            if !history.contains(&path) {
                history.push(path);
            }
            history
        });
    preferences
        .workspace_history
        .truncate(WORKSPACE_HISTORY_LIMIT);
}

/// ワークスペース選択をアプリ設定へ反映する
fn apply_workspace_selection(
    preferences: &mut AppPreferences,
    selection: WorkspaceSelection,
) -> Result<(), AppError> {
    if let Some(workspace_path) = selection.workspace_path {
        let workspace_path = workspace_path.trim();
        preferences.active_workspace_path = if workspace_path.is_empty() {
            None
        } else {
            Some(canonical_workspace_path(Path::new(workspace_path))?)
        };

        if let Some(workspace_path) = preferences.active_workspace_path.clone() {
            remember_workspace_path(preferences, workspace_path);
        }
    }

    if let Some(use_global) = selection.use_global {
        preferences.use_global = use_global;
    }

    if let Some(global_config_path) = selection.global_config_path {
        preferences.global_config_path = non_empty_path(Some(global_config_path));
    }

    Ok(())
}

/// ワークスペース履歴を更新する
fn remember_workspace_path(preferences: &mut AppPreferences, workspace_path: PathBuf) {
    preferences
        .workspace_history
        .retain(|existing| existing != &workspace_path);
    preferences.workspace_history.insert(0, workspace_path);
    preferences
        .workspace_history
        .truncate(WORKSPACE_HISTORY_LIMIT);
}

/// 存在するワークスペースの正規パスを取得する
fn canonical_workspace_path(path: &Path) -> Result<PathBuf, AppError> {
    let canonical = fs::canonicalize(path).map_err(|_| AppError::WorkspaceNotFound {
        path: path.to_path_buf(),
    })?;

    if !canonical.is_dir() {
        return Err(AppError::WorkspaceNotFound {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// 利用可能なワークスペースだけを正規化する
fn canonical_workspace_path_if_available(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path)
        .ok()
        .filter(|canonical| canonical.is_dir())
}

/// ワークスペース未選択時に読み込まれない local 設定パスを生成する
fn no_workspace_local_config_path(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join("no-workspace").join("fwd-deck.toml")
}

/// ワークスペース用の状態ファイルパスを生成する
fn workspace_state_file_path(app_config_dir: &Path, workspace_path: &Path) -> PathBuf {
    app_config_dir
        .join(WORKSPACE_STATES_DIR)
        .join(workspace_key(workspace_path))
        .join(STATE_FILE_NAME)
}

/// ワークスペースパスから安定した lower-hex key を生成する
fn workspace_key(workspace_path: &Path) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;

    for byte in workspace_path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    format!("{hash:016x}")
}

/// グローバル設定ファイルのパスを解決する
fn resolve_global_config_path(preferences: &AppPreferences) -> Result<Option<PathBuf>, AppError> {
    if !preferences.use_global {
        return Ok(None);
    }

    preferences
        .global_config_path
        .clone()
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
fn config_path_for_scope(
    paths: &RuntimePaths,
    kind: ConfigSourceKind,
) -> Result<PathBuf, AppError> {
    match kind {
        ConfigSourceKind::Global => paths
            .config_paths
            .global
            .clone()
            .ok_or(AppError::MissingGlobalConfigPath),
        ConfigSourceKind::Local => paths
            .local_config_path
            .clone()
            .ok_or(AppError::MissingWorkspace),
    }
}

/// 設定ファイル種別に対応する状態ファイルパスを取得する
fn state_path_for_source(paths: &RuntimePaths, kind: ConfigSourceKind) -> Result<&Path, AppError> {
    state_path_for_runtime_scope(paths, runtime_scope_for_source(kind))
}

/// runtime scope に対応する状態ファイルパスを取得する
fn state_path_for_runtime_scope(
    paths: &RuntimePaths,
    scope: RuntimeScope,
) -> Result<&Path, AppError> {
    match scope {
        RuntimeScope::Global => Ok(&paths.global_state_path),
        RuntimeScope::Workspace => paths
            .workspace_state_path
            .as_deref()
            .ok_or(AppError::MissingWorkspace),
    }
}

/// 設定ファイル種別に対応する runtime scope を取得する
fn runtime_scope_for_source(kind: ConfigSourceKind) -> RuntimeScope {
    match kind {
        ConfigSourceKind::Global => RuntimeScope::Global,
        ConfigSourceKind::Local => RuntimeScope::Workspace,
    }
}

/// runtime 状態ファイルから追跡中トンネルを読み込む
fn load_scoped_runtime_statuses(
    paths: &RuntimePaths,
) -> Result<Vec<ScopedRuntimeStatus>, AppError> {
    let mut statuses = tunnel_statuses(&paths.global_state_path)?
        .into_iter()
        .filter(|status| global_state_status_is_visible(paths, status))
        .map(|status| ScopedRuntimeStatus {
            runtime_scope: RuntimeScope::Global,
            status,
        })
        .collect::<Vec<_>>();

    if let Some(workspace_state_path) = &paths.workspace_state_path {
        statuses.extend(
            tunnel_statuses(workspace_state_path)?
                .into_iter()
                .filter(|status| status.state.source_kind == ConfigSourceKind::Local)
                .map(|status| ScopedRuntimeStatus {
                    runtime_scope: RuntimeScope::Workspace,
                    status,
                }),
        );
    }

    Ok(statuses)
}

/// 既定 state に保存された状態を現在の表示対象へ含めるか判定する
fn global_state_status_is_visible(paths: &RuntimePaths, status: &TunnelRuntimeStatus) -> bool {
    match status.state.source_kind {
        ConfigSourceKind::Global => true,
        ConfigSourceKind::Local => local_state_matches_active_workspace(paths, status),
    }
}

/// local state が現在のワークスペース設定に由来するか判定する
fn local_state_matches_active_workspace(
    paths: &RuntimePaths,
    status: &TunnelRuntimeStatus,
) -> bool {
    let Some(local_config_path) = &paths.local_config_path else {
        return false;
    };

    paths_refer_to_same_file(local_config_path, &status.state.source_path)
}

/// 2 つのパスが同じファイルを指すか比較する
fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// ダッシュボード状態へ変換する
fn build_dashboard_state(
    runtime_paths: RuntimePaths,
    config: EffectiveConfig,
    statuses: Vec<ScopedRuntimeStatus>,
    validation: ValidationReport,
) -> DashboardState {
    let status_by_key = statuses
        .iter()
        .map(|status| (runtime_status_lookup_key(status), status))
        .collect::<HashMap<_, _>>();
    let mut tunnels = config
        .tunnels
        .iter()
        .map(|resolved| {
            let runtime_key = (
                runtime_scope_for_source(resolved.source.kind),
                resolved.tunnel.id.as_str(),
            );
            tunnel_view(resolved, status_by_key.get(&runtime_key).copied())
        })
        .collect::<Vec<_>>();
    let mut tracked_tunnels = statuses.iter().map(tracked_tunnel_view).collect::<Vec<_>>();

    tunnels.sort_by(|left, right| left.id.cmp(&right.id));
    tracked_tunnels.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.runtime_scope.cmp(&right.runtime_scope))
    });

    DashboardState {
        paths: path_view(&runtime_paths),
        has_config: config.has_sources(),
        validation: validation_view(validation),
        tunnels,
        tracked_tunnels,
    }
}

/// runtime 状態を HashMap 検索用の借用キーへ変換する
fn runtime_status_lookup_key(status: &ScopedRuntimeStatus) -> (RuntimeScope, &str) {
    (
        runtime_scope_for_source(status.status.state.source_kind),
        status.status.state.id.as_str(),
    )
}

/// 解決済みパスを表示用へ変換する
fn path_view(paths: &RuntimePaths) -> PathView {
    PathView {
        workspace_path: paths
            .preferences
            .active_workspace_path
            .as_deref()
            .map(display_path)
            .unwrap_or_default(),
        workspace_history: paths
            .preferences
            .workspace_history
            .iter()
            .map(|path| display_path(path))
            .collect(),
        local_config_path: paths
            .local_config_path
            .as_deref()
            .map(display_path)
            .unwrap_or_default(),
        global_config_path: paths
            .global_config_display_path
            .as_deref()
            .map(display_path)
            .unwrap_or_default(),
        use_global: paths.preferences.use_global,
        global_state_path: display_path(&paths.global_state_path),
        workspace_state_path: paths
            .workspace_state_path
            .as_deref()
            .map(display_path)
            .unwrap_or_default(),
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
    status: Option<&ScopedRuntimeStatus>,
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
fn tracked_tunnel_view(status: &ScopedRuntimeStatus) -> TrackedTunnelView {
    let runtime_key = runtime_status_key(status.runtime_scope, &status.status.state.id);

    TrackedTunnelView {
        id: status.status.state.id.clone(),
        runtime_scope: status.runtime_scope,
        runtime_key,
        local: format_state_local_endpoint(&status.status),
        remote: format_state_remote_endpoint(&status.status),
        ssh: format_state_ssh_endpoint(&status.status),
        status: runtime_status_view(status),
    }
}

/// runtime 状態を表示用へ変換する
fn runtime_status_view(status: &ScopedRuntimeStatus) -> RuntimeStatusView {
    RuntimeStatusView {
        runtime_scope: status.runtime_scope,
        runtime_key: runtime_status_key(status.runtime_scope, &status.status.state.id),
        pid: status.status.state.pid,
        state: match status.status.process_state {
            ProcessState::Running => "running".to_owned(),
            ProcessState::Stale => "stale".to_owned(),
        },
        source: status.status.state.source_kind.to_string(),
        source_path: display_path(&status.status.state.source_path),
        started_at_unix_seconds: status.status.state.started_at_unix_seconds,
    }
}

/// runtime 状態の一意なキーを生成する
fn runtime_status_key(scope: RuntimeScope, id: &str) -> String {
    format!("{scope}:{id}")
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

    let mut existing_id = None;
    let mut existing_local_port = None;

    for resolved in &config.tunnels {
        if existing_id.is_none() && resolved.tunnel.id == tunnel.id {
            existing_id = Some(resolved);
        }

        if existing_local_port.is_none() && resolved.tunnel.local_port == tunnel.local_port {
            existing_local_port = Some(resolved);
        }

        if existing_id.is_some() && existing_local_port.is_some() {
            break;
        }
    }

    if let Some(existing) = existing_id {
        return Err(AppError::InvalidInput(format!(
            "同じ ID のトンネルが既に存在します: {} ({})",
            existing.tunnel.id,
            display_path(&existing.source.path)
        )));
    }

    if let Some(existing) = existing_local_port {
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

/// アプリの開始操作としてトンネルを開始する
fn start_tunnel_for_app(
    tunnel: &ResolvedTunnelConfig,
    state_path: &Path,
) -> Result<Option<String>, AppError> {
    match start_tunnel(tunnel, state_path) {
        Ok(started) => Ok(Some(start_success_message(started))),
        Err(TunnelRuntimeError::AlreadyRunning { id, pid }) => {
            Ok(Some(start_already_running_message(&id, pid)))
        }
        Err(error) => Err(AppError::Runtime(error)),
    }
}

/// アプリの停止操作としてトンネルを停止する
fn stop_tunnel_for_app(id: &str, state_path: &Path) -> Result<Option<String>, AppError> {
    match stop_tunnel(id, state_path) {
        Ok(stopped) => Ok(Some(stop_success_message(stopped))),
        Err(TunnelRuntimeError::NotTracked { id }) => Ok(Some(stop_already_stopped_message(&id))),
        Err(error) => Err(AppError::Runtime(error)),
    }
}

/// 複数トンネルに対する操作を順次実行する
fn run_tunnel_operations<F>(
    targets: &[OperationTargetInput],
    mut operation: F,
) -> Result<OperationReport, AppError>
where
    F: FnMut(&OperationTargetInput) -> Result<Option<String>, AppError>,
{
    if targets.is_empty() {
        return Err(AppError::InvalidInput(
            "操作対象のトンネルが選択されていません".to_owned(),
        ));
    }

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for target in targets {
        match operation(target) {
            Ok(Some(message)) => succeeded.push(OperationSuccessView {
                id: operation_target_label(target),
                message,
            }),
            Ok(None) => {}
            Err(error) => failed.push(OperationFailureView {
                id: operation_target_label(target),
                message: error.to_string(),
            }),
        }
    }

    Ok(OperationReport { succeeded, failed })
}

/// 操作対象の表示名を生成する
fn operation_target_label(target: &OperationTargetInput) -> String {
    match target.runtime_scope {
        Some(scope) => format!("{} ({scope})", target.id),
        None => target.id.clone(),
    }
}

/// 開始成功時のメッセージを生成する
fn start_success_message(started: StartedTunnel) -> String {
    format!(
        "{} を開始しました (pid: {})",
        started.state.id, started.state.pid
    )
}

/// 開始済みの場合の成功メッセージを生成する
fn start_already_running_message(id: &str, pid: u32) -> String {
    format!("{id} はすでに開始済みです (pid: {pid})")
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

/// 停止済みの場合の成功メッセージを生成する
fn stop_already_stopped_message(id: &str) -> String {
    format!("{id} はすでに停止済みです")
}

/// 統合済みトンネル設定を ID から参照する索引を生成する
fn tunnel_index_by_id(config: &EffectiveConfig) -> HashMap<&str, &ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .map(|resolved| (resolved.tunnel.id.as_str(), resolved))
        .collect()
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

#[cfg(test)]
mod tests {
    use fwd_deck_core::{ConfigSource, TunnelState, TunnelStateFile, state::write_state_file};
    use tempfile::TempDir;

    use super::*;

    /// preferences 未作成時の既定値を検証する
    #[test]
    fn missing_preferences_returns_defaults() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("preferences.toml");

        let preferences = read_preferences_file(&path).expect("read missing preferences");

        assert_eq!(preferences, AppPreferences::default());
    }

    /// ワークスペース選択時に履歴が先頭へ移動し重複しないことを検証する
    #[test]
    fn workspace_selection_updates_history_without_duplicates() {
        let first = TempDir::new().expect("create first workspace");
        let second = TempDir::new().expect("create second workspace");
        let mut preferences = AppPreferences::default();

        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(first.path().display().to_string()),
                global_config_path: None,
                use_global: None,
            },
        )
        .expect("select first workspace");
        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(second.path().display().to_string()),
                global_config_path: None,
                use_global: None,
            },
        )
        .expect("select second workspace");
        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(first.path().display().to_string()),
                global_config_path: None,
                use_global: None,
            },
        )
        .expect("select first workspace again");

        assert_eq!(
            preferences.active_workspace_path.as_deref(),
            Some(
                fs::canonicalize(first.path())
                    .expect("canonical first")
                    .as_path()
            )
        );
        assert_eq!(preferences.workspace_history.len(), 2);
        assert_eq!(
            preferences.workspace_history[0],
            fs::canonicalize(first.path()).expect("canonical first")
        );
    }

    /// ワークスペース履歴が上限件数に制限されることを検証する
    #[test]
    fn workspace_history_is_limited_to_ten_entries() {
        let workspaces = (0..12)
            .map(|_| TempDir::new().expect("create workspace"))
            .collect::<Vec<_>>();
        let mut preferences = AppPreferences::default();

        for workspace in &workspaces {
            apply_workspace_selection(
                &mut preferences,
                WorkspaceSelection {
                    workspace_path: Some(workspace.path().display().to_string()),
                    global_config_path: None,
                    use_global: None,
                },
            )
            .expect("select workspace");
        }

        assert_eq!(preferences.workspace_history.len(), WORKSPACE_HISTORY_LIMIT);
    }

    /// local 設定未作成のワークスペースでも local パスが解決されることを検証する
    #[test]
    fn workspace_local_config_path_does_not_require_existing_file() {
        let workspace = TempDir::new().expect("create workspace");

        let local_config_path = default_local_config_path(workspace.path());

        assert_eq!(local_config_path, workspace.path().join("fwd-deck.toml"));
        assert!(!local_config_path.exists());
    }

    /// 起動済みトンネルがアプリ操作では成功扱いになることを検証する
    #[test]
    fn start_tunnel_for_app_reports_already_running_tunnel_as_success() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");
        let tunnel = resolved_tunnel("db", temp_dir.path().join("fwd-deck.toml"));
        let pid = std::process::id();
        let mut state_file = TunnelStateFile::new();
        state_file.upsert(TunnelState::from_resolved_tunnel(
            &tunnel,
            pid,
            1_700_000_000,
        ));
        write_state_file(&state_path, &state_file).expect("write state file");

        let message =
            start_tunnel_for_app(&tunnel, &state_path).expect("report already running tunnel");

        assert_eq!(
            message,
            Some(format!("db はすでに開始済みです (pid: {pid})"))
        );
    }

    /// 未追跡の停止対象がアプリ操作では成功扱いになることを検証する
    #[test]
    fn stop_tunnel_for_app_reports_untracked_tunnel_as_success() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");

        let message =
            stop_tunnel_for_app("missing", &state_path).expect("report already stopped tunnel");

        assert_eq!(message, Some("missing はすでに停止済みです".to_owned()));
    }

    /// スキップされた操作対象が成功件数と失敗件数に含まれないことを検証する
    #[test]
    fn run_tunnel_operations_omits_skipped_targets() {
        let targets = vec![OperationTargetInput {
            id: "missing".to_owned(),
            runtime_scope: None,
        }];

        let report =
            run_tunnel_operations(&targets, |_target| Ok::<Option<String>, AppError>(None))
                .expect("run operation");

        assert!(report.succeeded.is_empty());
        assert!(report.failed.is_empty());
    }

    /// 設定ファイル種別に応じて runtime scope が分かれることを検証する
    #[test]
    fn runtime_scope_matches_config_source_kind() {
        assert_eq!(
            runtime_scope_for_source(ConfigSourceKind::Global),
            RuntimeScope::Global
        );
        assert_eq!(
            runtime_scope_for_source(ConfigSourceKind::Local),
            RuntimeScope::Workspace
        );
    }

    /// runtime key が scope と ID で一意化されることを検証する
    #[test]
    fn runtime_status_key_includes_scope_and_id() {
        assert_eq!(runtime_status_key(RuntimeScope::Global, "db"), "global:db");
        assert_eq!(
            runtime_status_key(RuntimeScope::Workspace, "db"),
            "workspace:db"
        );
    }

    /// CLI 既定 state の local 状態が現在のワークスペース表示に含まれることを検証する
    #[test]
    fn global_state_local_status_is_visible_for_active_workspace() {
        let workspace = TempDir::new().expect("create workspace");
        let local_config_path = workspace.path().join("fwd-deck.toml");
        fs::write(&local_config_path, "").expect("write local config");
        let paths = runtime_paths_for_local_config(local_config_path.clone());
        let status = runtime_status(ConfigSourceKind::Local, local_config_path);

        assert!(global_state_status_is_visible(&paths, &status));
    }

    /// 別ワークスペース由来の local 状態が表示対象から除外されることを検証する
    #[test]
    fn global_state_local_status_is_hidden_for_other_workspace() {
        let active_workspace = TempDir::new().expect("create active workspace");
        let other_workspace = TempDir::new().expect("create other workspace");
        let active_config_path = active_workspace.path().join("fwd-deck.toml");
        let other_config_path = other_workspace.path().join("fwd-deck.toml");
        fs::write(&active_config_path, "").expect("write active config");
        fs::write(&other_config_path, "").expect("write other config");
        let paths = runtime_paths_for_local_config(active_config_path);
        let status = runtime_status(ConfigSourceKind::Local, other_config_path);

        assert!(!global_state_status_is_visible(&paths, &status));
    }

    /// 状態の関連付けキーが設定ファイル種別に従うことを検証する
    #[test]
    fn runtime_status_lookup_key_uses_config_source_kind() {
        let status = ScopedRuntimeStatus {
            runtime_scope: RuntimeScope::Global,
            status: runtime_status(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml")),
        };

        assert_eq!(
            runtime_status_lookup_key(&status),
            (RuntimeScope::Workspace, "db")
        );
    }

    /// ワークスペース state path が app config directory 配下に生成されることを検証する
    #[test]
    fn workspace_state_path_uses_stable_lower_hex_key() {
        let app_config = TempDir::new().expect("create app config");
        let workspace = TempDir::new().expect("create workspace");

        let state_path = workspace_state_file_path(app_config.path(), workspace.path());
        let key = state_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .expect("state path should contain workspace key");

        assert_eq!(key.len(), 16);
        assert!(key.chars().all(|character| character.is_ascii_hexdigit()));
        assert!(key.chars().all(|character| !character.is_ascii_uppercase()));
        assert!(state_path.ends_with(STATE_FILE_NAME));
    }

    /// テスト用の runtime paths を生成する
    fn runtime_paths_for_local_config(local_config_path: PathBuf) -> RuntimePaths {
        RuntimePaths {
            preferences: AppPreferences::default(),
            config_paths: ConfigPaths::new(None, local_config_path.clone()),
            local_config_path: Some(local_config_path),
            global_config_display_path: None,
            global_state_path: PathBuf::from("global-state.toml"),
            workspace_state_path: None,
        }
    }

    /// テスト用の runtime status を生成する
    fn runtime_status(source_kind: ConfigSourceKind, source_path: PathBuf) -> TunnelRuntimeStatus {
        TunnelRuntimeStatus {
            state: TunnelState {
                id: "db".to_owned(),
                pid: 1000,
                local_host: "127.0.0.1".to_owned(),
                local_port: 15432,
                remote_host: "127.0.0.1".to_owned(),
                remote_port: 5432,
                ssh_user: "user".to_owned(),
                ssh_host: "bastion.example.com".to_owned(),
                ssh_port: None,
                source_kind,
                source_path,
                started_at_unix_seconds: 1_700_000_000,
            },
            process_state: ProcessState::Running,
        }
    }

    /// テスト用の resolved tunnel を生成する
    fn resolved_tunnel(id: &str, source_path: PathBuf) -> ResolvedTunnelConfig {
        ResolvedTunnelConfig::new(
            ConfigSource::new(ConfigSourceKind::Local, source_path),
            TunnelConfig {
                id: id.to_owned(),
                description: None,
                tags: Vec::new(),
                local_host: None,
                local_port: 15432,
                remote_host: "127.0.0.1".to_owned(),
                remote_port: 5432,
                ssh_user: "user".to_owned(),
                ssh_host: "bastion.example.com".to_owned(),
                ssh_port: None,
                identity_file: None,
                timeouts: TimeoutConfig::default(),
            },
        )
    }
}
