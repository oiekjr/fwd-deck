use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(target_os = "macos")]
use std::{thread, time::Duration};

use fwd_deck_core::{
    ConfigEditError, ConfigLoadError, ConfigPaths, ConfigSourceKind, EffectiveConfig, ProcessState,
    ResolvedTimeoutConfig, ResolvedTunnelConfig, StartedTunnel, StoppedTunnel, TimeoutConfig,
    TunnelConfig, TunnelRuntimeError, TunnelRuntimeStatus, ValidationReport,
    add_tunnel_to_config_file, default_global_config_path, default_local_config_path,
    default_state_file_path, load_effective_config, remove_tunnel_from_config_file,
    start_tunnels_with_progress, stop_tunnel, tag_is_valid, tunnel_statuses, validate_config,
};
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApp, NSImage};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSData, NSProcessInfo, NSString};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use tauri::menu::AboutMetadata;
use tauri::{
    Emitter, Manager,
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use thiserror::Error;

const APP_PREFERENCES_FILE_NAME: &str = "preferences.toml";
const APP_PREFERENCES_VERSION: u32 = 3;
const WORKSPACE_HISTORY_LIMIT: usize = 10;
const WORKSPACE_STATES_DIR: &str = "workspace-states";
const STATE_FILE_NAME: &str = "state.toml";
const START_TUNNELS_PARALLELISM: usize = 4;
const OPERATION_PROGRESS_EVENT: &str = "operation-progress";
const TRAY_OPERATION_RESULT_EVENT: &str = "tray-operation-result";
const OPEN_SETTINGS_EVENT: &str = "open-settings";
const MAIN_WINDOW_LABEL: &str = "main";
const APP_MENU_SETTINGS: &str = "app-settings";
const TRAY_ID: &str = "main-tray";
const TRAY_ICON_IDLE_BYTES: &[u8] = include_bytes!("../icons/tray-idle-template.png");
const TRAY_ICON_ACTIVE_BYTES: &[u8] = include_bytes!("../icons/tray-active-template.png");
const TRAY_MENU_SHOW: &str = "tray-show";
const TRAY_MENU_HIDE: &str = "tray-hide";
const TRAY_MENU_SETTINGS: &str = "tray-settings";
const TRAY_MENU_HIDE_DOCK_WHEN_HIDDEN: &str = "tray-hide-dock-when-hidden";
const TRAY_MENU_REFRESH: &str = "tray-refresh";
const TRAY_MENU_QUIT: &str = "tray-quit";
const TRAY_MENU_CURRENT_WORKSPACE: &str = "tray-current-workspace";
const TRAY_MENU_WORKSPACE_BROWSE: &str = "tray-workspace-browse";
const TRAY_MENU_NO_TUNNELS: &str = "tray-no-tunnels";
const TRAY_MENU_INVALID_CONFIG: &str = "tray-invalid-config";
const TRAY_TUNNEL_ITEM_PREFIX: &str = "tray-tunnel-";
const TRAY_WORKSPACE_ITEM_PREFIX: &str = "tray-workspace-";
const TRAY_OPERATION_ID: &str = "tray";
const APP_DISPLAY_NAME: &str = "Fwd Deck";
const QUIT_DIALOG_TITLE: &str = "Fwd Deck を終了";
const QUIT_DIALOG_STOP_LABEL: &str = "停止して終了";
const QUIT_DIALOG_KEEP_LABEL: &str = "停止せず終了";
const QUIT_DIALOG_CANCEL_LABEL: &str = "キャンセル";
const QUIT_ERROR_TITLE: &str = "ポートフォワーディングを停止できませんでした";
const QUIT_STALE_CLEANUP_ERROR_TITLE: &str = "stale 状態を削除できませんでした";

/// Tauri アプリを起動する
fn main() {
    set_runtime_application_name();

    let quit_state = QuitConfirmationStateHandle::default();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(OperationLockState::default())
        .manage(TrayState::default())
        .menu(build_app_menu)
        .on_menu_event(handle_app_menu_event)
        .setup(|app| {
            set_runtime_dock_icon();
            initialize_tray(app.handle())
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_dashboard,
            switch_workspace,
            start_tunnels,
            stop_tunnels,
            add_tunnel_entry,
            remove_tunnel_entry,
            remove_workspace_history_entry,
            refresh_tray_menu
        ])
        .build(tauri::generate_context!())
        .expect("error while running Fwd Deck application");

    app.run(move |app, event| {
        handle_quit_confirmation_event(app, event, quit_state.clone());
    });
}

/// 開発実行時の macOS 表示名を設定する
#[cfg(target_os = "macos")]
fn set_runtime_application_name() {
    NSProcessInfo::processInfo().setProcessName(&NSString::from_str(APP_DISPLAY_NAME));
}

/// 開発実行時の macOS 表示名を設定する
#[cfg(not(target_os = "macos"))]
fn set_runtime_application_name() {}

/// start / stop 操作の同時実行を防ぐ状態を保持する
#[derive(Debug, Default)]
struct OperationLockState(Mutex<()>);

/// トレイアイコンと動的メニュー操作を保持する
#[derive(Default)]
struct TrayState {
    icon: Mutex<Option<TrayIcon>>,
    tunnel_actions: Mutex<HashMap<String, TrayTunnelAction>>,
    workspace_actions: Mutex<HashMap<String, TrayWorkspaceAction>>,
}

impl TrayState {
    /// トレイアイコンを保存する
    fn set_icon(&self, icon: TrayIcon) {
        *self
            .icon
            .lock()
            .expect("tray icon state should not be poisoned") = Some(icon);
    }

    /// トレイアイコンを取得する
    fn icon(&self) -> Option<TrayIcon> {
        self.icon
            .lock()
            .expect("tray icon state should not be poisoned")
            .clone()
    }

    /// トレイメニューの動的操作を差し替える
    fn set_tunnel_actions(&self, tunnel_actions: HashMap<String, TrayTunnelAction>) {
        *self
            .tunnel_actions
            .lock()
            .expect("tray action state should not be poisoned") = tunnel_actions;
    }

    /// トレイメニューの動的操作を差し替える
    fn set_actions(&self, actions: TrayMenuActions) {
        self.set_tunnel_actions(actions.tunnel_actions);
        *self
            .workspace_actions
            .lock()
            .expect("tray workspace action state should not be poisoned") =
            actions.workspace_actions;
    }

    /// トレイメニュー ID に対応する操作を取得する
    fn tunnel_action(&self, menu_id: &str) -> Option<TrayTunnelAction> {
        self.tunnel_actions
            .lock()
            .expect("tray action state should not be poisoned")
            .get(menu_id)
            .cloned()
    }

    /// トレイメニュー ID に対応するワークスペース操作を取得する
    fn workspace_action(&self, menu_id: &str) -> Option<TrayWorkspaceAction> {
        self.workspace_actions
            .lock()
            .expect("tray workspace action state should not be poisoned")
            .get(menu_id)
            .cloned()
    }
}

/// トレイから実行するトンネル操作を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayTunnelAction {
    id: String,
    runtime_scope: Option<RuntimeScope>,
    operation: TrayTunnelOperation,
}

/// トレイから実行する start / stop を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayTunnelOperation {
    Start,
    Stop,
}

/// トレイメニューへ表示するトンネル項目を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayTunnelMenuItem {
    menu_id: String,
    label: String,
    checked: bool,
    enabled: bool,
    action: TrayTunnelAction,
}

/// トレイメニューの動的操作対応表を表現する
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrayMenuActions {
    tunnel_actions: HashMap<String, TrayTunnelAction>,
    workspace_actions: HashMap<String, TrayWorkspaceAction>,
}

/// トレイアイコンへ反映する接続状態を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayIconKind {
    Idle,
    Active,
}

/// トレイから実行するワークスペース操作を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayWorkspaceAction {
    workspace_path: PathBuf,
}

/// トレイメニューへ表示するワークスペース項目を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayWorkspaceMenuItem {
    menu_id: String,
    label: String,
    checked: bool,
    enabled: bool,
    action: Option<TrayWorkspaceAction>,
}

/// トレイ操作結果をフロントエンドへ通知する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrayOperationResultEvent {
    kind: String,
    summary: String,
    detail: Option<String>,
}

/// アプリ上部メニューを作成する
fn build_app_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;

    #[cfg(target_os = "macos")]
    if let Some(app_submenu) = menu
        .items()?
        .first()
        .and_then(|item| item.as_submenu().cloned())
    {
        let about = PredefinedMenuItem::about(app, None, Some(app_about_metadata(app)?))?;
        let settings = MenuItem::with_id(
            app,
            APP_MENU_SETTINGS,
            "Settings...",
            true,
            Some("CmdOrCtrl+,"),
        )?;
        let separator = PredefinedMenuItem::separator(app)?;

        let _ = app_submenu.remove_at(0)?;
        app_submenu.insert(&about, 0)?;
        app_submenu.insert(&settings, 2)?;
        app_submenu.insert(&separator, 3)?;
    }

    Ok(menu)
}

/// About パネルへ表示するアプリ情報を生成する
#[cfg(target_os = "macos")]
fn app_about_metadata(app: &tauri::AppHandle) -> tauri::Result<AboutMetadata<'static>> {
    let package_info = app.package_info();
    let config = app.config();
    let icon_bytes = include_bytes!("../icons/128x128@2x.png");

    Ok(AboutMetadata {
        name: Some(package_info.name.clone()),
        version: Some(package_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config
            .bundle
            .publisher
            .clone()
            .map(|publisher| vec![publisher]),
        icon: Some(Image::from_bytes(icon_bytes)?.to_owned()),
        ..Default::default()
    })
}

/// アプリ上部メニューの選択を処理する
fn handle_app_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    if event.id().as_ref() != APP_MENU_SETTINGS {
        return;
    }

    open_settings_window(app);
}

/// Settings 表示をフロントエンドへ要求する
fn open_settings_window(app: &tauri::AppHandle) {
    let _ = show_main_window(app);
    let _ = app.emit(OPEN_SETTINGS_EVENT, ());
}

/// 開発実行時も Dock にアプリのアイコンを表示する
#[cfg(target_os = "macos")]
fn set_runtime_dock_icon() {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };

    let icon_bytes = include_bytes!("../icons/128x128@2x.png");
    let data =
        unsafe { NSData::dataWithBytes_length(icon_bytes.as_ptr().cast(), icon_bytes.len()) };
    let Some(image) = NSImage::initWithData(main_thread.alloc(), &data) else {
        return;
    };

    unsafe {
        NSApp(main_thread).setApplicationIconImage(Some(&image));
    }
}

/// Dock アイコン復元を行わずに処理を完了する
#[cfg(not(target_os = "macos"))]
fn set_runtime_dock_icon() {}

/// トレイアイコンと初期メニューを作成する
fn initialize_tray(app: &tauri::AppHandle) -> Result<(), AppError> {
    let (menu, actions, icon_kind) = build_tray_menu(app)?;
    let tray_icon_image = tray_icon_image(icon_kind)?;
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .icon(tray_icon_image)
        .icon_as_template(true)
        .tooltip(APP_DISPLAY_NAME)
        .show_menu_on_left_click(true)
        .on_menu_event(handle_tray_menu_event)
        .on_tray_icon_event(handle_tray_icon_event);

    let tray_icon = builder.build(app)?;
    let tray_state = app.state::<TrayState>();
    tray_state.set_actions(actions);
    tray_state.set_icon(tray_icon);

    Ok(())
}

/// トレイアイコン操作時にメニュー状態を更新する
fn handle_tray_icon_event(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        let _ = rebuild_tray_menu(tray.app_handle());
    }
}

/// トレイメニューの選択を実行する
fn handle_tray_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    let menu_id = event.id().as_ref();

    match menu_id {
        TRAY_MENU_SHOW => handle_tray_result(app, show_main_window(app)),
        TRAY_MENU_HIDE => handle_tray_result(app, hide_window_to_tray(app, MAIN_WINDOW_LABEL)),
        TRAY_MENU_SETTINGS => open_settings_window(app),
        TRAY_MENU_HIDE_DOCK_WHEN_HIDDEN => handle_tray_dock_visibility_toggle(app),
        TRAY_MENU_REFRESH => handle_tray_result(app, rebuild_tray_menu(app)),
        TRAY_MENU_WORKSPACE_BROWSE => handle_tray_workspace_browse(app),
        TRAY_MENU_QUIT => app.exit(0),
        _ => {
            let tray_state = app.state::<TrayState>();
            if let Some(action) = tray_state.tunnel_action(menu_id) {
                handle_tray_tunnel_action(app, action);
            } else if let Some(action) = tray_state.workspace_action(menu_id) {
                handle_tray_workspace_action(app, action);
            }
        }
    }
}

/// トレイからの単体トンネル操作を実行する
fn handle_tray_tunnel_action(app: &tauri::AppHandle, action: TrayTunnelAction) {
    let operation_lock = app.state::<OperationLockState>();
    let result = with_operation_lock(&operation_lock, || {
        run_tray_tunnel_action(app, action.clone())
    });

    let _ = rebuild_tray_menu(app);

    match result {
        Ok(report) => emit_tray_operation_report(app, report),
        Err(error) => emit_tray_operation_error(app, error.to_string()),
    }
}

/// トレイの単体操作を既存 start / stop 処理へ委譲する
fn run_tray_tunnel_action(
    app: &tauri::AppHandle,
    action: TrayTunnelAction,
) -> Result<OperationReport, AppError> {
    let target = OperationTargetInput {
        id: action.id,
        runtime_scope: action.runtime_scope,
    };

    match action.operation {
        TrayTunnelOperation::Start => {
            start_tunnels_inner(app, None, vec![target], TRAY_OPERATION_ID)
        }
        TrayTunnelOperation::Stop => stop_tunnels_inner(app, None, vec![target], TRAY_OPERATION_ID),
    }
}

/// トレイからのワークスペース切り替えを実行する
fn handle_tray_workspace_action(app: &tauri::AppHandle, action: TrayWorkspaceAction) {
    let result = switch_tray_workspace(app, action.workspace_path);
    emit_tray_workspace_result(app, result);
}

/// トレイからワークスペース選択ダイアログを表示する
fn handle_tray_workspace_browse(app: &tauri::AppHandle) {
    let dialog = app.dialog().file().set_title("Select Workspace");
    let dialog = match tray_workspace_dialog_directory(app) {
        Ok(Some(path)) => dialog.set_directory(path),
        Ok(None) => dialog,
        Err(error) => {
            emit_tray_operation_error(app, error.to_string());
            return;
        }
    };
    let app = app.clone();

    dialog.pick_folder(move |workspace_path| {
        let Some(workspace_path) = workspace_path else {
            return;
        };

        let result = workspace_path
            .into_path()
            .map_err(|error| {
                AppError::InvalidInput(format!("ワークスペースパスを解決できませんでした: {error}"))
            })
            .and_then(|workspace_path| switch_tray_workspace(&app, workspace_path));

        emit_tray_workspace_result(&app, result);
    });
}

/// ワークスペース切り替え結果をメニューとフロントエンドへ反映する
fn emit_tray_workspace_result(app: &tauri::AppHandle, result: Result<PathBuf, AppError>) {
    let _ = rebuild_tray_menu(app);

    match result {
        Ok(workspace_path) => {
            let _ = app.emit(
                TRAY_OPERATION_RESULT_EVENT,
                TrayOperationResultEvent {
                    kind: "success".to_owned(),
                    summary: "Workspace を切り替えました".to_owned(),
                    detail: Some(display_path(&workspace_path)),
                },
            );
        }
        Err(error) => emit_tray_operation_error(app, error.to_string()),
    }
}

/// トレイ操作からワークスペース設定を保存する
fn switch_tray_workspace(
    app: &tauri::AppHandle,
    workspace_path: PathBuf,
) -> Result<PathBuf, AppError> {
    let selection = workspace_selection_for_path(&workspace_path)?;
    let operation_lock = app.state::<OperationLockState>();
    let (runtime_paths, _) = with_operation_lock(&operation_lock, || {
        switch_workspace_runtime_paths_for_app(app, selection)
    })?;

    Ok(runtime_paths
        .preferences
        .active_workspace_path
        .unwrap_or(workspace_path))
}

/// ワークスペース選択ダイアログの開始ディレクトリを決定する
fn tray_workspace_dialog_directory(app: &tauri::AppHandle) -> Result<Option<PathBuf>, AppError> {
    let app_config_dir = app_config_dir(app)?;
    let preferences_path = preferences_path_from_app_config_dir(&app_config_dir);
    let mut preferences = read_preferences_file(&preferences_path)?;

    normalize_loaded_preferences(&mut preferences);

    Ok(preferences
        .active_workspace_path
        .or_else(|| preferences.workspace_history.first().cloned()))
}

/// Path をワークスペース選択入力へ変換する
fn workspace_selection_for_path(path: &Path) -> Result<WorkspaceSelection, AppError> {
    let Some(workspace_path) = path.to_str() else {
        return Err(AppError::InvalidInput(
            "ワークスペースパスに非 UTF-8 文字が含まれています".to_owned(),
        ));
    };

    Ok(WorkspaceSelection {
        workspace_path: Some(workspace_path.to_owned()),
        ..WorkspaceSelection::default()
    })
}

/// トレイ操作の成功・失敗をフロントエンドへ通知する
fn emit_tray_operation_report(app: &tauri::AppHandle, report: OperationReport) {
    if let Some(event) = tray_operation_event_from_report(report) {
        if event.kind == "error" {
            let _ = show_main_window(app);
        }

        let _ = app.emit(TRAY_OPERATION_RESULT_EVENT, event);
    }
}

/// トレイ操作の失敗をフロントエンドへ通知する
fn emit_tray_operation_error(app: &tauri::AppHandle, message: String) {
    let _ = show_main_window(app);
    let _ = app.emit(
        TRAY_OPERATION_RESULT_EVENT,
        TrayOperationResultEvent {
            kind: "error".to_owned(),
            summary: message,
            detail: None,
        },
    );
}

/// トレイから Dock 表示設定を切り替える
fn handle_tray_dock_visibility_toggle(app: &tauri::AppHandle) {
    match toggle_hidden_window_dock_preference(app) {
        Ok(should_hide) => {
            let _ = rebuild_tray_menu(app);
            let summary = if should_hide {
                "ウィンドウ非表示中は Dock アイコンを隠します"
            } else {
                "ウィンドウ非表示中も Dock アイコンを表示します"
            };

            let _ = app.emit(
                TRAY_OPERATION_RESULT_EVENT,
                TrayOperationResultEvent {
                    kind: "success".to_owned(),
                    summary: summary.to_owned(),
                    detail: None,
                },
            );
        }
        Err(error) => emit_tray_operation_error(app, error.to_string()),
    }
}

/// トレイ操作結果を通知イベントへ変換する
fn tray_operation_event_from_report(report: OperationReport) -> Option<TrayOperationResultEvent> {
    let success_count = report.succeeded.len();
    let failure_count = report.failed.len();

    if success_count == 0 && failure_count == 0 {
        return None;
    }

    if failure_count == 0 {
        return Some(TrayOperationResultEvent {
            kind: "success".to_owned(),
            summary: format!("{success_count} 件の操作が完了しました"),
            detail: None,
        });
    }

    Some(TrayOperationResultEvent {
        kind: if success_count > 0 { "info" } else { "error" }.to_owned(),
        summary: format!("{success_count} 件成功、{failure_count} 件失敗しました"),
        detail: Some(
            report
                .failed
                .into_iter()
                .map(|failure| format!("{}: {}", failure.id, failure.message))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    })
}

/// トレイ操作の補助処理結果を通知する
fn handle_tray_result(app: &tauri::AppHandle, result: Result<(), AppError>) {
    if let Err(error) = result {
        emit_tray_operation_error(app, error.to_string());
    }
}

/// トレイメニューを現在の設定と状態で再構築する
#[tauri::command]
fn refresh_tray_menu(app: tauri::AppHandle) -> Result<(), String> {
    command_result(rebuild_tray_menu(&app))
}

/// トレイメニューを現在の設定と状態で再構築する
fn rebuild_tray_menu(app: &tauri::AppHandle) -> Result<(), AppError> {
    let (menu, actions, icon_kind) = build_tray_menu(app)?;
    let tray_icon_image = tray_icon_image(icon_kind)?;
    app.state::<TrayState>().set_actions(actions);

    if let Some(icon) = app.state::<TrayState>().icon() {
        icon.set_menu(Some(menu))?;
        icon.set_icon(Some(tray_icon_image))?;
        icon.set_icon_as_template(true)?;
    }

    Ok(())
}

/// トレイメニューと動的項目の対応表を生成する
fn build_tray_menu(
    app: &tauri::AppHandle,
) -> Result<(Menu<tauri::Wry>, TrayMenuActions, TrayIconKind), AppError> {
    let runtime_paths = resolve_runtime_paths(app, None)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let statuses = load_scoped_runtime_statuses(&runtime_paths)?;
    let validation = validate_config(&config);
    let tunnel_items = tray_tunnel_menu_items(&config, &statuses, &validation);
    let workspace_items = tray_workspace_menu_items(&runtime_paths.preferences);
    let icon_kind = tray_icon_kind(&statuses);
    let mut actions = TrayMenuActions::default();

    let menu = Menu::new(app)?;
    let show = MenuItem::with_id(app, TRAY_MENU_SHOW, "Open Fwd Deck", true, None::<&str>)?;
    let settings = MenuItem::with_id(
        app,
        TRAY_MENU_SETTINGS,
        "Settings...",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let refresh = MenuItem::with_id(app, TRAY_MENU_REFRESH, "Refresh Status", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "Quit", true, None::<&str>)?;
    let tunnels = Submenu::new(app, "Tunnels", true)?;
    let workspaces = Submenu::new(app, "Workspaces", true)?;

    if tunnel_items.is_empty() {
        let empty = MenuItem::with_id(
            app,
            TRAY_MENU_NO_TUNNELS,
            "No tunnels configured",
            false,
            None::<&str>,
        )?;
        tunnels.append(&empty)?;
    } else {
        for item in tunnel_items {
            let menu_item = CheckMenuItem::with_id(
                app,
                item.menu_id.clone(),
                item.label,
                item.enabled,
                item.checked,
                None::<&str>,
            )?;
            actions.tunnel_actions.insert(item.menu_id, item.action);
            tunnels.append(&menu_item)?;
        }
    }

    if !validation.is_valid() {
        let invalid = MenuItem::with_id(
            app,
            TRAY_MENU_INVALID_CONFIG,
            "Config has errors",
            false,
            None::<&str>,
        )?;
        tunnels.append(&PredefinedMenuItem::separator(app)?)?;
        tunnels.append(&invalid)?;
    }

    for item in workspace_items {
        let menu_item = CheckMenuItem::with_id(
            app,
            item.menu_id.clone(),
            item.label,
            item.enabled,
            item.checked,
            None::<&str>,
        )?;

        if let Some(action) = item.action {
            actions.workspace_actions.insert(item.menu_id, action);
        }

        workspaces.append(&menu_item)?;
    }

    let browse_workspace = MenuItem::with_id(
        app,
        TRAY_MENU_WORKSPACE_BROWSE,
        "Browse Workspace...",
        true,
        None::<&str>,
    )?;
    workspaces.append(&PredefinedMenuItem::separator(app)?)?;
    workspaces.append(&browse_workspace)?;

    menu.append(&tunnels)?;
    menu.append(&refresh)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&workspaces)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&settings)?;
    menu.append(&show)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;

    Ok((menu, actions, icon_kind))
}

/// 接続状態からトレイアイコン種別を決定する
fn tray_icon_kind(statuses: &[ScopedRuntimeStatus]) -> TrayIconKind {
    if statuses
        .iter()
        .any(|status| status.status.process_state == ProcessState::Running)
    {
        TrayIconKind::Active
    } else {
        TrayIconKind::Idle
    }
}

/// トレイアイコン種別に対応する画像を読み込む
fn tray_icon_image(kind: TrayIconKind) -> Result<Image<'static>, AppError> {
    let bytes = match kind {
        TrayIconKind::Idle => TRAY_ICON_IDLE_BYTES,
        TrayIconKind::Active => TRAY_ICON_ACTIVE_BYTES,
    };

    Ok(Image::from_bytes(bytes)?.to_owned())
}

/// 設定済みトンネルをトレイメニュー項目へ変換する
fn tray_tunnel_menu_items(
    config: &EffectiveConfig,
    statuses: &[ScopedRuntimeStatus],
    validation: &ValidationReport,
) -> Vec<TrayTunnelMenuItem> {
    let status_by_key = statuses
        .iter()
        .map(|status| (runtime_status_lookup_key(status), status))
        .collect::<HashMap<_, _>>();
    let can_start = validation.is_valid();
    let mut items = config
        .tunnels
        .iter()
        .enumerate()
        .map(|(index, resolved)| {
            let runtime_key = (
                runtime_scope_for_source(resolved.source.kind),
                resolved.tunnel.id.as_str(),
            );
            let status = status_by_key.get(&runtime_key).copied();
            tray_tunnel_menu_item(index, resolved, status, can_start)
        })
        .collect::<Vec<_>>();

    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

/// 保存済みワークスペースをトレイメニュー項目へ変換する
fn tray_workspace_menu_items(preferences: &AppPreferences) -> Vec<TrayWorkspaceMenuItem> {
    let mut items = Vec::new();

    items.push(match &preferences.active_workspace_path {
        Some(workspace_path) => TrayWorkspaceMenuItem {
            menu_id: TRAY_MENU_CURRENT_WORKSPACE.to_owned(),
            label: format!("Current workspace: {}", display_path(workspace_path)),
            checked: true,
            enabled: false,
            action: None,
        },
        None => TrayWorkspaceMenuItem {
            menu_id: TRAY_MENU_CURRENT_WORKSPACE.to_owned(),
            label: "No workspace selected".to_owned(),
            checked: false,
            enabled: false,
            action: None,
        },
    });

    let mut index = 0;
    for workspace_path in &preferences.workspace_history {
        if preferences.active_workspace_path.as_ref() == Some(workspace_path) {
            continue;
        }

        items.push(TrayWorkspaceMenuItem {
            menu_id: format!("{TRAY_WORKSPACE_ITEM_PREFIX}{index}"),
            label: display_path(workspace_path),
            checked: false,
            enabled: true,
            action: Some(TrayWorkspaceAction {
                workspace_path: workspace_path.clone(),
            }),
        });
        index += 1;
    }

    items
}

/// 1 件のトンネルをトレイメニュー項目へ変換する
fn tray_tunnel_menu_item(
    index: usize,
    resolved: &ResolvedTunnelConfig,
    status: Option<&ScopedRuntimeStatus>,
    can_start: bool,
) -> TrayTunnelMenuItem {
    let is_running = status
        .map(|status| status.status.process_state == ProcessState::Running)
        .unwrap_or(false);
    let is_stale = status
        .map(|status| status.status.process_state == ProcessState::Stale)
        .unwrap_or(false);
    let operation = if is_running {
        TrayTunnelOperation::Stop
    } else {
        TrayTunnelOperation::Start
    };
    let runtime_scope = match operation {
        TrayTunnelOperation::Start => None,
        TrayTunnelOperation::Stop => status.map(|status| status.runtime_scope),
    };

    TrayTunnelMenuItem {
        menu_id: format!("{TRAY_TUNNEL_ITEM_PREFIX}{index}"),
        label: tray_tunnel_label(&resolved.tunnel.id, is_stale),
        checked: is_running,
        enabled: is_running || can_start,
        action: TrayTunnelAction {
            id: resolved.tunnel.id.clone(),
            runtime_scope,
            operation,
        },
    }
}

/// トレイ表示用のトンネル名を生成する
fn tray_tunnel_label(id: &str, is_stale: bool) -> String {
    if is_stale {
        format!("{id} (stale)")
    } else {
        id.to_owned()
    }
}

/// start / stop 操作を直列化して実行する
fn with_operation_lock<T, F>(
    operation_lock: &OperationLockState,
    operation: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError>,
{
    let _guard = operation_lock
        .0
        .lock()
        .expect("operation lock should not be poisoned");

    operation()
}

/// メインウィンドウを表示する
fn show_main_window(app: &tauri::AppHandle) -> Result<(), AppError> {
    set_dock_visibility(app, true)?;

    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

/// メインウィンドウをトレイへ隠す
fn hide_window_to_tray(app: &tauri::AppHandle, label: &str) -> Result<(), AppError> {
    if let Some(window) = app.get_webview_window(label) {
        window.hide()?;
    }

    apply_hidden_window_dock_visibility(app)
}

/// 非表示中の Dock 表示設定を反映する
fn apply_hidden_window_dock_visibility(app: &tauri::AppHandle) -> Result<(), AppError> {
    let runtime_paths = resolve_runtime_paths(app, None)?;
    set_dock_visibility(
        app,
        !runtime_paths.preferences.hide_dock_icon_when_window_hidden,
    )
}

/// 非表示中の Dock 表示設定を保存して現在状態へ反映する
fn toggle_hidden_window_dock_preference(app: &tauri::AppHandle) -> Result<bool, AppError> {
    let app_config_dir = app_config_dir(app)?;
    let preferences_path = preferences_path_from_app_config_dir(&app_config_dir);
    let mut preferences = read_preferences_file(&preferences_path)?;

    normalize_loaded_preferences(&mut preferences);
    preferences.hide_dock_icon_when_window_hidden = !preferences.hide_dock_icon_when_window_hidden;

    let runtime_paths = runtime_paths_from_preferences(&app_config_dir, preferences)?;
    write_preferences_file(&preferences_path, &runtime_paths.preferences)?;
    apply_window_state_dock_visibility(
        app,
        runtime_paths.preferences.hide_dock_icon_when_window_hidden,
    )?;

    Ok(runtime_paths.preferences.hide_dock_icon_when_window_hidden)
}

/// 現在のウィンドウ表示状態に応じて Dock 表示設定を反映する
fn apply_window_state_dock_visibility(
    app: &tauri::AppHandle,
    hide_when_window_hidden: bool,
) -> Result<(), AppError> {
    let is_window_visible = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .map(|window| window.is_visible())
        .transpose()?
        .unwrap_or(false);

    set_dock_visibility(app, is_window_visible || !hide_when_window_hidden)
}

/// Dock 表示状態を切り替える
#[cfg(target_os = "macos")]
fn set_dock_visibility(app: &tauri::AppHandle, visible: bool) -> Result<(), AppError> {
    app.set_dock_visibility(visible)?;
    if visible {
        restore_runtime_dock_icon(app)?;
    }

    Ok(())
}

/// Dock 再表示後に実行時アイコンを復元する
#[cfg(target_os = "macos")]
fn restore_runtime_dock_icon(app: &tauri::AppHandle) -> Result<(), AppError> {
    app.run_on_main_thread(set_runtime_dock_icon)?;

    // Dock の再表示は macOS 側で非同期に完了するため、完了後にもアイコンを再適用する
    let app = app.clone();
    let _ = thread::Builder::new()
        .name("dock-icon-restore".to_owned())
        .spawn(move || {
            thread::sleep(Duration::from_millis(250));
            let _ = app.run_on_main_thread(set_runtime_dock_icon);
        });

    Ok(())
}

/// Dock 表示状態を切り替える
#[cfg(not(target_os = "macos"))]
fn set_dock_visibility(_app: &tauri::AppHandle, _visible: bool) -> Result<(), AppError> {
    Ok(())
}

/// 終了確認の状態を共有する
#[derive(Debug, Clone, Default)]
struct QuitConfirmationStateHandle(Arc<Mutex<QuitConfirmationState>>);

impl QuitConfirmationStateHandle {
    /// 終了確認の状態を取得する
    fn get(&self) -> QuitConfirmationState {
        *self
            .0
            .lock()
            .expect("quit confirmation state should not be poisoned")
    }

    /// 終了確認の状態を更新する
    fn set(&self, state: QuitConfirmationState) {
        *self
            .0
            .lock()
            .expect("quit confirmation state should not be poisoned") = state;
    }
}

/// 終了確認の進行状態を表現する
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum QuitConfirmationState {
    #[default]
    Idle,
    Prompting,
    Proceeding,
}

/// ユーザーが要求した終了種別を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
enum QuitRequest {
    AppExit,
}

/// 終了確認ダイアログの選択結果を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitDialogAction {
    StopAndQuit,
    QuitOnly,
    Cancel,
}

/// 終了時に停止または削除するトンネルを表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuitTunnelTarget {
    id: String,
    runtime_scope: RuntimeScope,
    state_path: PathBuf,
    process_state: ProcessState,
}

impl QuitTunnelTarget {
    /// 終了処理の表示用 ID を生成する
    fn display_id(&self) -> String {
        runtime_status_key(self.runtime_scope, &self.id)
    }
}

/// 終了時停止処理の失敗を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuitStopFailure {
    id: String,
    message: String,
}

/// 終了時に扱うトンネルを状態別に保持する
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QuitTunnelTargets {
    running: Vec<QuitTunnelTarget>,
    stale: Vec<QuitTunnelTarget>,
}

/// 終了時に起動中トンネルを扱う方針を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitRunningTunnelsAction {
    AutoStop,
    Prompt,
}

/// 終了時の対象とアプリ設定を保持する
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuitTunnelContext {
    targets: QuitTunnelTargets,
    running_action: QuitRunningTunnelsAction,
}

/// 終了イベントに応じて停止確認を制御する
fn handle_quit_confirmation_event(
    app: &tauri::AppHandle,
    event: tauri::RunEvent,
    quit_state: QuitConfirmationStateHandle,
) {
    match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            handle_exit_requested(app, api, quit_state);
        }
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            handle_close_requested(app, api, &label);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            handle_reopen_requested(app);
        }
        _ => {}
    }
}

/// アプリ再起動要求に応じてメインウィンドウを復帰する
#[cfg(target_os = "macos")]
fn handle_reopen_requested(app: &tauri::AppHandle) {
    let _ = show_main_window(app);
}

/// アプリ終了要求に対する停止確認を開始する
fn handle_exit_requested(
    app: &tauri::AppHandle,
    api: tauri::ExitRequestApi,
    quit_state: QuitConfirmationStateHandle,
) {
    if should_prevent_quit(app, quit_state, QuitRequest::AppExit) {
        api.prevent_exit();
    }
}

/// ウィンドウ終了要求に対する停止確認を開始する
fn handle_close_requested(app: &tauri::AppHandle, api: tauri::CloseRequestApi, label: &str) {
    api.prevent_close();
    let _ = hide_window_to_tray(app, label);
}

/// 終了を一時停止して確認を出す必要があるか判定する
fn should_prevent_quit(
    app: &tauri::AppHandle,
    quit_state: QuitConfirmationStateHandle,
    request: QuitRequest,
) -> bool {
    match quit_state.get() {
        QuitConfirmationState::Proceeding => return false,
        QuitConfirmationState::Prompting => return true,
        QuitConfirmationState::Idle => quit_state.set(QuitConfirmationState::Prompting),
    }

    match collect_quit_tunnel_context(app) {
        Ok(context) => handle_collected_quit_context(app, quit_state, request, context),
        Err(error) => {
            show_quit_error_dialog(
                app.clone(),
                quit_state,
                QUIT_ERROR_TITLE,
                format!("終了前の状態確認に失敗しました。\n\n{error}"),
            );
            true
        }
    }
}

/// 収集した終了対象と設定に応じて自動掃除、停止、確認を行う
fn handle_collected_quit_context(
    app: &tauri::AppHandle,
    quit_state: QuitConfirmationStateHandle,
    request: QuitRequest,
    context: QuitTunnelContext,
) -> bool {
    let targets = context.targets;

    if let Err(failure) = stop_quit_tunnel_targets(&targets.stale) {
        show_quit_error_dialog(
            app.clone(),
            quit_state,
            QUIT_STALE_CLEANUP_ERROR_TITLE,
            quit_stale_cleanup_failure_message(&failure),
        );
        return true;
    }

    if targets.running.is_empty() {
        quit_state.set(QuitConfirmationState::Idle);
        return false;
    }

    match context.running_action {
        QuitRunningTunnelsAction::AutoStop => match stop_quit_tunnel_targets(&targets.running) {
            Ok(()) => perform_confirmed_quit(app.clone(), quit_state, request),
            Err(failure) => show_quit_error_dialog(
                app.clone(),
                quit_state,
                QUIT_ERROR_TITLE,
                quit_stop_failure_message(&failure),
            ),
        },
        QuitRunningTunnelsAction::Prompt => {
            show_quit_confirmation_dialog(app.clone(), quit_state, request, targets.running);
        }
    }

    true
}

/// 終了時に停止または削除する対象と設定を収集する
fn collect_quit_tunnel_context(app: &tauri::AppHandle) -> Result<QuitTunnelContext, AppError> {
    let runtime_paths = resolve_runtime_paths(app, None)?;
    let targets = collect_visible_quit_tunnel_targets(&runtime_paths)?;
    let running_action = quit_running_tunnels_action(&runtime_paths.preferences);

    Ok(QuitTunnelContext {
        targets,
        running_action,
    })
}

/// アプリ設定から終了時の起動中トンネル処理を決定する
fn quit_running_tunnels_action(preferences: &AppPreferences) -> QuitRunningTunnelsAction {
    if preferences.auto_stop_tunnels_on_quit {
        QuitRunningTunnelsAction::AutoStop
    } else {
        QuitRunningTunnelsAction::Prompt
    }
}

/// 表示中スコープの tracked tunnel を終了処理対象へ変換する
fn collect_visible_quit_tunnel_targets(
    paths: &RuntimePaths,
) -> Result<QuitTunnelTargets, AppError> {
    let statuses = load_scoped_runtime_statuses(paths)?;
    quit_tunnel_targets_from_statuses(paths, &statuses)
}

/// runtime 状態を終了処理対象へ変換する
fn quit_tunnel_targets_from_statuses(
    paths: &RuntimePaths,
    statuses: &[ScopedRuntimeStatus],
) -> Result<QuitTunnelTargets, AppError> {
    let mut targets = QuitTunnelTargets::default();

    for status in statuses {
        let state_path = state_path_for_runtime_scope(paths, status.runtime_scope)?;
        let target = QuitTunnelTarget {
            id: status.status.state.id.clone(),
            runtime_scope: status.runtime_scope,
            state_path: state_path.to_path_buf(),
            process_state: status.status.process_state,
        };

        match status.status.process_state {
            ProcessState::Running => targets.running.push(target),
            ProcessState::Stale => targets.stale.push(target),
        }
    }

    Ok(targets)
}

/// 終了確認ダイアログを表示する
fn show_quit_confirmation_dialog(
    app: tauri::AppHandle,
    quit_state: QuitConfirmationStateHandle,
    request: QuitRequest,
    targets: Vec<QuitTunnelTarget>,
) {
    let app_for_callback = app.clone();

    app.dialog()
        .message(quit_confirmation_message())
        .title(QUIT_DIALOG_TITLE)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            QUIT_DIALOG_STOP_LABEL.to_owned(),
            QUIT_DIALOG_KEEP_LABEL.to_owned(),
            QUIT_DIALOG_CANCEL_LABEL.to_owned(),
        ))
        .show_with_result(move |result| {
            handle_quit_dialog_result(
                app_for_callback,
                quit_state,
                request,
                targets,
                quit_dialog_action(&result),
            );
        });
}

/// 終了確認ダイアログの本文を生成する
fn quit_confirmation_message() -> &'static str {
    "起動中のポートフォワーディングがあります。\n停止して終了しますか？"
}

/// 終了確認ダイアログの結果を内部処理へ変換する
fn quit_dialog_action(result: &MessageDialogResult) -> QuitDialogAction {
    match result {
        MessageDialogResult::Yes => QuitDialogAction::StopAndQuit,
        MessageDialogResult::No => QuitDialogAction::QuitOnly,
        MessageDialogResult::Custom(label) if label == QUIT_DIALOG_STOP_LABEL => {
            QuitDialogAction::StopAndQuit
        }
        MessageDialogResult::Custom(label) if label == QUIT_DIALOG_KEEP_LABEL => {
            QuitDialogAction::QuitOnly
        }
        _ => QuitDialogAction::Cancel,
    }
}

/// 終了確認ダイアログの選択結果を実行する
fn handle_quit_dialog_result(
    app: tauri::AppHandle,
    quit_state: QuitConfirmationStateHandle,
    request: QuitRequest,
    targets: Vec<QuitTunnelTarget>,
    action: QuitDialogAction,
) {
    match action {
        QuitDialogAction::StopAndQuit => match stop_quit_tunnel_targets(&targets) {
            Ok(()) => perform_confirmed_quit(app, quit_state, request),
            Err(failure) => show_quit_error_dialog(
                app,
                quit_state,
                QUIT_ERROR_TITLE,
                quit_stop_failure_message(&failure),
            ),
        },
        QuitDialogAction::QuitOnly => perform_confirmed_quit(app, quit_state, request),
        QuitDialogAction::Cancel => quit_state.set(QuitConfirmationState::Idle),
    }
}

/// 終了前に対象トンネルを停止または stale 削除する
fn stop_quit_tunnel_targets(targets: &[QuitTunnelTarget]) -> Result<(), QuitStopFailure> {
    for target in targets {
        if let Err(error) = stop_tunnel_for_app(&target.id, &target.state_path) {
            return Err(QuitStopFailure {
                id: target.display_id(),
                message: error.to_string(),
            });
        }
    }

    Ok(())
}

/// 停止失敗時の表示メッセージを生成する
fn quit_stop_failure_message(failure: &QuitStopFailure) -> String {
    format!(
        "{} の停止に失敗したため、終了を中止しました。\n\n{}",
        failure.id, failure.message
    )
}

/// stale 掃除失敗時の表示メッセージを生成する
fn quit_stale_cleanup_failure_message(failure: &QuitStopFailure) -> String {
    format!(
        "{} の stale 状態を削除できなかったため、終了を中止しました。\n\n{}",
        failure.id, failure.message
    )
}

/// 確認済みの終了要求を再実行する
fn perform_confirmed_quit(
    app: tauri::AppHandle,
    quit_state: QuitConfirmationStateHandle,
    request: QuitRequest,
) {
    quit_state.set(QuitConfirmationState::Proceeding);

    match request {
        QuitRequest::AppExit => app.exit(0),
    }
}

/// 終了処理のエラーダイアログを表示する
fn show_quit_error_dialog(
    app: tauri::AppHandle,
    quit_state: QuitConfirmationStateHandle,
    title: &str,
    message: String,
) {
    app.dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::Ok)
        .show(move |_| {
            quit_state.set(QuitConfirmationState::Idle);
        });
}

/// フロントエンドから指定するワークスペース選択を表現する
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceSelection {
    workspace_path: Option<String>,
    global_config_path: Option<String>,
    use_global: Option<bool>,
    hide_dock_icon_when_window_hidden: Option<bool>,
    auto_stop_tunnels_on_quit: Option<bool>,
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
    hide_dock_icon_when_window_hidden: bool,
    auto_stop_tunnels_on_quit: bool,
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
            hide_dock_icon_when_window_hidden: false,
            auto_stop_tunnels_on_quit: false,
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
    hide_dock_icon_when_window_hidden: bool,
    auto_stop_tunnels_on_quit: bool,
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

/// ワークスペース切り替え結果を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceSwitchResult {
    dashboard: DashboardState,
    stopped_count: usize,
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

/// フロントエンドへ通知する一括操作の進捗を表現する
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationProgressEvent {
    operation_id: String,
    completed_count: usize,
    total_count: usize,
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

/// 一括操作の進捗通知を送信する
struct OperationProgressEmitter<'a> {
    app: &'a tauri::AppHandle,
    operation_id: &'a str,
    total_count: usize,
    completed_count: usize,
}

impl<'a> OperationProgressEmitter<'a> {
    /// 一括操作の進捗通知を初期化する
    fn new(app: &'a tauri::AppHandle, operation_id: &'a str, total_count: usize) -> Self {
        Self {
            app,
            operation_id,
            total_count,
            completed_count: 0,
        }
    }

    /// 完了件数を 1 件進めて通知する
    fn advance(&mut self) {
        self.completed_count = self.completed_count.saturating_add(1).min(self.total_count);
        self.emit_current();
    }

    /// 現在の進捗をフロントエンドへ通知する
    fn emit_current(&self) {
        let _ = self.app.emit(
            OPERATION_PROGRESS_EVENT,
            OperationProgressEvent {
                operation_id: self.operation_id.to_owned(),
                completed_count: self.completed_count,
                total_count: self.total_count,
            },
        );
    }
}

/// 開始対象の入力順と解決済み設定を保持する
#[derive(Debug, Clone)]
struct StartOperationTarget {
    index: usize,
    target: OperationTargetInput,
    tunnel: ResolvedTunnelConfig,
}

/// 同一 state file に対する開始対象を保持する
#[derive(Debug, Clone)]
struct StartOperationGroup {
    state_path: PathBuf,
    targets: Vec<StartOperationTarget>,
}

/// 一括操作 1 件の結果を保持する
#[derive(Debug, Clone)]
enum OperationOutcome {
    Succeeded(OperationSuccessView),
    Failed(OperationFailureView),
    Skipped,
}

/// runtime scope を付与したトンネル状態を表現する
#[derive(Debug, Clone)]
struct ScopedRuntimeStatus {
    runtime_scope: RuntimeScope,
    status: TunnelRuntimeStatus,
}

/// ワークスペース切り替え前に停止するトンネルを表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceSwitchStopTarget {
    id: String,
    runtime_scope: RuntimeScope,
    state_path: PathBuf,
}

impl WorkspaceSwitchStopTarget {
    /// 表示用 ID を生成する
    fn display_id(&self) -> String {
        runtime_status_key(self.runtime_scope, &self.id)
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
    #[error("旧ワークスペースのポートフォワーディングを停止できませんでした: {id}: {message}")]
    WorkspaceSwitchStop { id: String, message: String },
    #[error("アプリ操作に失敗しました: {0}")]
    Tauri(#[from] tauri::Error),
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

/// ワークスペースを切り替えて旧ワークスペース由来のトンネルを停止する
#[tauri::command]
fn switch_workspace(
    app: tauri::AppHandle,
    operation_lock: tauri::State<'_, OperationLockState>,
    paths: WorkspaceSelection,
) -> Result<WorkspaceSwitchResult, String> {
    let result = with_operation_lock(&operation_lock, || switch_workspace_inner(&app, paths));
    let _ = rebuild_tray_menu(&app);

    command_result(result)
}

/// 指定トンネルを開始する
#[tauri::command]
fn start_tunnels(
    app: tauri::AppHandle,
    operation_lock: tauri::State<'_, OperationLockState>,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
    operation_id: String,
) -> Result<OperationReport, String> {
    let result = with_operation_lock(&operation_lock, || {
        start_tunnels_inner(&app, paths, targets, &operation_id)
    });
    let _ = rebuild_tray_menu(&app);

    command_result(result)
}

/// 指定トンネルを停止する
#[tauri::command]
fn stop_tunnels(
    app: tauri::AppHandle,
    operation_lock: tauri::State<'_, OperationLockState>,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
    operation_id: String,
) -> Result<OperationReport, String> {
    let result = with_operation_lock(&operation_lock, || {
        stop_tunnels_inner(&app, paths, targets, &operation_id)
    });
    let _ = rebuild_tray_menu(&app);

    command_result(result)
}

/// 設定ファイルへトンネルを追加する
#[tauri::command]
fn add_tunnel_entry(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    scope: ConfigScopeInput,
    tunnel: TunnelInput,
) -> Result<DashboardState, String> {
    let result = add_tunnel_entry_inner(&app, paths, scope, tunnel);
    let _ = rebuild_tray_menu(&app);

    command_result(result)
}

/// 設定ファイルからトンネルを削除する
#[tauri::command]
fn remove_tunnel_entry(
    app: tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    scope: ConfigScopeInput,
    id: String,
) -> Result<DashboardState, String> {
    let result = remove_tunnel_entry_inner(&app, paths, scope, &id);
    let _ = rebuild_tray_menu(&app);

    command_result(result)
}

/// ワークスペース履歴から指定パスを削除する
#[tauri::command]
fn remove_workspace_history_entry(
    app: tauri::AppHandle,
    workspace_path: String,
) -> Result<PathView, String> {
    command_result(remove_workspace_history_entry_inner(&app, &workspace_path))
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

    load_dashboard_from_runtime_paths(runtime_paths)
}

/// 解決済みパスからダッシュボード状態を組み立てる
fn load_dashboard_from_runtime_paths(
    runtime_paths: RuntimePaths,
) -> Result<DashboardState, AppError> {
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

/// ワークスペース切り替え処理を実行する
fn switch_workspace_inner(
    app: &tauri::AppHandle,
    selection: WorkspaceSelection,
) -> Result<WorkspaceSwitchResult, AppError> {
    let (runtime_paths, stopped_count) = switch_workspace_runtime_paths_for_app(app, selection)?;
    let dashboard = load_dashboard_from_runtime_paths(runtime_paths)?;

    Ok(WorkspaceSwitchResult {
        dashboard,
        stopped_count,
    })
}

/// トンネル開始処理を実行する
fn start_tunnels_inner(
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
    operation_id: &str,
) -> Result<OperationReport, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let tunnels_by_id = tunnel_index_by_id(&config);

    ensure_valid_config(&config)?;
    let mut progress = OperationProgressEmitter::new(app, operation_id, targets.len());
    run_start_tunnel_operations(&runtime_paths, &targets, &tunnels_by_id, &mut progress)
}

/// トンネル停止処理を実行する
fn stop_tunnels_inner(
    app: &tauri::AppHandle,
    paths: Option<WorkspaceSelection>,
    targets: Vec<OperationTargetInput>,
    operation_id: &str,
) -> Result<OperationReport, AppError> {
    let runtime_paths = resolve_runtime_paths(app, paths)?;
    let config = load_effective_config(&runtime_paths.config_paths)?;
    let tunnels_by_id = tunnel_index_by_id(&config);
    let mut progress = OperationProgressEmitter::new(app, operation_id, targets.len());

    run_tunnel_operations_with_progress(
        &targets,
        |target| {
            let state_path = match target.runtime_scope {
                Some(scope) => state_path_for_runtime_scope(&runtime_paths, scope)?,
                None => {
                    let tunnel =
                        tunnels_by_id
                            .get(target.id.as_str())
                            .copied()
                            .ok_or_else(|| {
                                AppError::InvalidInput(format!(
                                    "未定義のトンネル ID です: {}",
                                    target.id
                                ))
                            })?;
                    state_path_for_source(&runtime_paths, tunnel.source.kind)?
                }
            };

            stop_tunnel_for_app(&target.id, state_path)
        },
        |_target| progress.advance(),
    )
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

/// ワークスペース履歴の削除結果を表示用パスとして返す
fn remove_workspace_history_entry_inner(
    app: &tauri::AppHandle,
    workspace_path: &str,
) -> Result<PathView, AppError> {
    let app_config_dir = app_config_dir(app)?;
    let preferences_path = preferences_path_from_app_config_dir(&app_config_dir);
    let mut preferences = read_preferences_file(&preferences_path)?;

    normalize_loaded_preferences(&mut preferences);
    remove_workspace_history_entry_from_preferences(&mut preferences, workspace_path)?;

    let runtime_paths = runtime_paths_from_preferences(&app_config_dir, preferences)?;
    write_preferences_file(&preferences_path, &runtime_paths.preferences)?;

    Ok(path_view(&runtime_paths))
}

/// アプリ設定ディレクトリに基づいてワークスペース切り替え後のパスを解決する
fn switch_workspace_runtime_paths_for_app(
    app: &tauri::AppHandle,
    selection: WorkspaceSelection,
) -> Result<(RuntimePaths, usize), AppError> {
    let app_config_dir = app_config_dir(app)?;
    let preferences_path = preferences_path_from_app_config_dir(&app_config_dir);

    switch_workspace_runtime_paths(&app_config_dir, &preferences_path, selection)
}

/// 旧ワークスペース由来のトンネルを停止してから設定を保存する
fn switch_workspace_runtime_paths(
    app_config_dir: &Path,
    preferences_path: &Path,
    selection: WorkspaceSelection,
) -> Result<(RuntimePaths, usize), AppError> {
    apply_workspace_switch_with_stop(
        app_config_dir,
        preferences_path,
        selection,
        stop_previous_workspace_tunnels,
    )
}

/// 停止処理を注入してワークスペース設定を切り替える
fn apply_workspace_switch_with_stop<F>(
    app_config_dir: &Path,
    preferences_path: &Path,
    selection: WorkspaceSelection,
    stop_previous_workspace: F,
) -> Result<(RuntimePaths, usize), AppError>
where
    F: FnOnce(&RuntimePaths) -> Result<usize, AppError>,
{
    let mut preferences = read_preferences_file(preferences_path)?;
    let original_preferences = preferences.clone();

    normalize_loaded_preferences(&mut preferences);

    let previous_preferences = preferences.clone();
    let mut next_preferences = preferences;
    apply_workspace_selection(&mut next_preferences, selection)?;

    let stopped_count = if workspace_path_changed(&previous_preferences, &next_preferences) {
        let previous_runtime_paths =
            runtime_paths_from_preferences(app_config_dir, previous_preferences)?;
        stop_previous_workspace(&previous_runtime_paths)?
    } else {
        0
    };

    let runtime_paths = runtime_paths_from_preferences(app_config_dir, next_preferences)?;
    write_preferences_file_if_changed(
        preferences_path,
        &original_preferences,
        &runtime_paths.preferences,
    )?;

    Ok((runtime_paths, stopped_count))
}

/// active workspace の変更有無を判定する
fn workspace_path_changed(previous: &AppPreferences, next: &AppPreferences) -> bool {
    previous.active_workspace_path != next.active_workspace_path
}

/// アプリ設定と入力から実行時パスを解決する
fn resolve_runtime_paths(
    app: &tauri::AppHandle,
    selection: Option<WorkspaceSelection>,
) -> Result<RuntimePaths, AppError> {
    let app_config_dir = app_config_dir(app)?;
    let preferences_path = preferences_path_from_app_config_dir(&app_config_dir);
    let mut preferences = read_preferences_file(&preferences_path)?;
    let original_preferences = preferences.clone();

    normalize_loaded_preferences(&mut preferences);

    if let Some(selection) = selection {
        apply_workspace_selection(&mut preferences, selection)?;
    }

    let runtime_paths = runtime_paths_from_preferences(&app_config_dir, preferences)?;
    write_preferences_file_if_changed(
        &preferences_path,
        &original_preferences,
        &runtime_paths.preferences,
    )?;

    Ok(runtime_paths)
}

/// アプリ設定から実行時パスを組み立てる
fn runtime_paths_from_preferences(
    app_config_dir: &Path,
    preferences: AppPreferences,
) -> Result<RuntimePaths, AppError> {
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
        .unwrap_or_else(|| no_workspace_local_config_path(app_config_dir));
    let global_state_path = default_state_file_path().ok_or(AppError::MissingStatePath)?;
    let workspace_state_path = preferences
        .active_workspace_path
        .as_deref()
        .map(|workspace| workspace_state_file_path(app_config_dir, workspace));

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

/// 変更がある場合だけアプリ設定ファイルを書き込む
fn write_preferences_file_if_changed(
    path: &Path,
    original: &AppPreferences,
    next: &AppPreferences,
) -> Result<(), AppError> {
    if original == next {
        return Ok(());
    }

    write_preferences_file(path, next)
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
        let previous_workspace_path = preferences.active_workspace_path.clone();
        preferences.active_workspace_path = if workspace_path.is_empty() {
            None
        } else {
            Some(canonical_workspace_path(Path::new(workspace_path))?)
        };

        if preferences.active_workspace_path != previous_workspace_path
            && let Some(workspace_path) = preferences.active_workspace_path.clone()
        {
            remember_workspace_path(preferences, workspace_path);
        }
    }

    if let Some(use_global) = selection.use_global {
        preferences.use_global = use_global;
    }

    if let Some(global_config_path) = selection.global_config_path {
        preferences.global_config_path = non_empty_path(Some(global_config_path));
    }

    if let Some(hide_dock_icon_when_window_hidden) = selection.hide_dock_icon_when_window_hidden {
        preferences.hide_dock_icon_when_window_hidden = hide_dock_icon_when_window_hidden;
    }

    if let Some(auto_stop_tunnels_on_quit) = selection.auto_stop_tunnels_on_quit {
        preferences.auto_stop_tunnels_on_quit = auto_stop_tunnels_on_quit;
    }

    Ok(())
}

/// 指定したワークスペースを履歴から削除する
fn remove_workspace_history_entry_from_preferences(
    preferences: &mut AppPreferences,
    workspace_path: &str,
) -> Result<(), AppError> {
    let workspace_path = workspace_path.trim();
    if workspace_path.is_empty() {
        return Err(AppError::InvalidInput(
            "削除対象のワークスペースパスが空です".to_owned(),
        ));
    }

    if let Some(workspace_path) = canonical_workspace_path_if_available(Path::new(workspace_path)) {
        preferences
            .workspace_history
            .retain(|existing| existing != &workspace_path);
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

/// 旧ワークスペース由来のトンネルを停止する
fn stop_previous_workspace_tunnels(paths: &RuntimePaths) -> Result<usize, AppError> {
    let targets = collect_workspace_switch_stop_targets(paths)?;

    stop_workspace_switch_targets(&targets)
}

/// 旧ワークスペース由来の停止対象を収集する
fn collect_workspace_switch_stop_targets(
    paths: &RuntimePaths,
) -> Result<Vec<WorkspaceSwitchStopTarget>, AppError> {
    let Some(local_config_path) = &paths.local_config_path else {
        return Ok(Vec::new());
    };
    let mut targets = collect_workspace_switch_stop_targets_from_state(
        &paths.global_state_path,
        RuntimeScope::Global,
        local_config_path,
    )?;

    if let Some(workspace_state_path) = &paths.workspace_state_path {
        targets.extend(collect_workspace_switch_stop_targets_from_state(
            workspace_state_path,
            RuntimeScope::Workspace,
            local_config_path,
        )?);
    }

    Ok(targets)
}

/// 指定 state から旧ワークスペース local 設定由来の停止対象を収集する
fn collect_workspace_switch_stop_targets_from_state(
    state_path: &Path,
    runtime_scope: RuntimeScope,
    local_config_path: &Path,
) -> Result<Vec<WorkspaceSwitchStopTarget>, AppError> {
    Ok(tunnel_statuses(state_path)?
        .into_iter()
        .filter(|status| status.state.source_kind == ConfigSourceKind::Local)
        .filter(|status| paths_refer_to_same_file(local_config_path, &status.state.source_path))
        .map(|status| WorkspaceSwitchStopTarget {
            id: status.state.id,
            runtime_scope,
            state_path: state_path.to_path_buf(),
        })
        .collect())
}

/// 旧ワークスペース切り替え対象を停止する
fn stop_workspace_switch_targets(targets: &[WorkspaceSwitchStopTarget]) -> Result<usize, AppError> {
    for target in targets {
        stop_tunnel_for_app(&target.id, &target.state_path).map_err(|error| {
            AppError::WorkspaceSwitchStop {
                id: target.display_id(),
                message: error.to_string(),
            }
        })?;
    }

    Ok(targets.len())
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
        hide_dock_icon_when_window_hidden: paths.preferences.hide_dock_icon_when_window_hidden,
        auto_stop_tunnels_on_quit: paths.preferences.auto_stop_tunnels_on_quit,
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

/// アプリの一括開始操作としてトンネルを開始する
fn run_start_tunnel_operations(
    paths: &RuntimePaths,
    targets: &[OperationTargetInput],
    tunnels_by_id: &HashMap<&str, &ResolvedTunnelConfig>,
    progress: &mut OperationProgressEmitter<'_>,
) -> Result<OperationReport, AppError> {
    ensure_operation_targets_selected(targets)?;

    let mut outcomes = (0..targets.len()).map(|_| None).collect::<Vec<_>>();
    let mut groups = Vec::new();

    for (index, target) in targets.iter().enumerate() {
        match resolve_start_operation_target(paths, tunnels_by_id, index, target) {
            Ok((state_path, operation_target)) => {
                push_start_operation_group(&mut groups, state_path, operation_target);
            }
            Err(error) => {
                outcomes[index] = Some(operation_failure_outcome(target, error.to_string()));
                progress.advance();
            }
        }
    }

    for group in groups {
        let tunnels = group
            .targets
            .iter()
            .map(|target| target.tunnel.clone())
            .collect::<Vec<_>>();
        let mut reported_count = 0;

        match start_tunnels_with_progress(
            &tunnels,
            &group.state_path,
            START_TUNNELS_PARALLELISM,
            |_index, _result| {
                reported_count += 1;
                progress.advance();
            },
        ) {
            Ok(results) => {
                for (target, result) in group.targets.into_iter().zip(results) {
                    outcomes[target.index] =
                        Some(start_operation_result_outcome(&target.target, result));
                }
            }
            Err(error) => {
                let message = AppError::Runtime(error).to_string();
                let remaining_count = group.targets.len().saturating_sub(reported_count);
                for target in group.targets {
                    outcomes[target.index] =
                        Some(operation_failure_outcome(&target.target, message.clone()));
                }
                for _ in 0..remaining_count {
                    progress.advance();
                }
            }
        }
    }

    Ok(operation_report_from_outcomes(outcomes))
}

/// 開始操作の入力を状態ファイル単位の実行対象へ変換する
fn resolve_start_operation_target(
    paths: &RuntimePaths,
    tunnels_by_id: &HashMap<&str, &ResolvedTunnelConfig>,
    index: usize,
    target: &OperationTargetInput,
) -> Result<(PathBuf, StartOperationTarget), AppError> {
    let tunnel = tunnels_by_id
        .get(target.id.as_str())
        .copied()
        .ok_or_else(|| {
            AppError::InvalidInput(format!("未定義のトンネル ID です: {}", target.id))
        })?;
    let state_path = state_path_for_source(paths, tunnel.source.kind)?.to_path_buf();

    Ok((
        state_path,
        StartOperationTarget {
            index,
            target: target.clone(),
            tunnel: (*tunnel).clone(),
        },
    ))
}

/// 同一 state file の開始対象を同じ実行グループへ追加する
fn push_start_operation_group(
    groups: &mut Vec<StartOperationGroup>,
    state_path: PathBuf,
    target: StartOperationTarget,
) {
    if let Some(group) = groups
        .iter_mut()
        .find(|group| group.state_path == state_path)
    {
        group.targets.push(target);
        return;
    }

    groups.push(StartOperationGroup {
        state_path,
        targets: vec![target],
    });
}

/// 開始結果をアプリ表示用の一括操作結果へ変換する
fn start_operation_result_outcome(
    target: &OperationTargetInput,
    result: Result<StartedTunnel, TunnelRuntimeError>,
) -> OperationOutcome {
    match start_tunnel_result_for_app(result) {
        Ok(Some(message)) => OperationOutcome::Succeeded(OperationSuccessView {
            id: operation_target_label(target),
            message,
        }),
        Ok(None) => OperationOutcome::Skipped,
        Err(error) => operation_failure_outcome(target, error.to_string()),
    }
}

/// 単体開始結果をアプリ表示用メッセージへ変換する
fn start_tunnel_result_for_app(
    result: Result<StartedTunnel, TunnelRuntimeError>,
) -> Result<Option<String>, AppError> {
    match result {
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

/// 複数トンネルに対する操作を順次実行して結果確定時に通知する
fn run_tunnel_operations_with_progress<F, G>(
    targets: &[OperationTargetInput],
    mut operation: F,
    mut on_result: G,
) -> Result<OperationReport, AppError>
where
    F: FnMut(&OperationTargetInput) -> Result<Option<String>, AppError>,
    G: FnMut(&OperationTargetInput),
{
    ensure_operation_targets_selected(targets)?;

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

        on_result(target);
    }

    Ok(OperationReport { succeeded, failed })
}

/// 操作対象が空でないことを検証する
fn ensure_operation_targets_selected(targets: &[OperationTargetInput]) -> Result<(), AppError> {
    if targets.is_empty() {
        return Err(AppError::InvalidInput(
            "操作対象のトンネルが選択されていません".to_owned(),
        ));
    }

    Ok(())
}

/// 失敗した操作結果を生成する
fn operation_failure_outcome(target: &OperationTargetInput, message: String) -> OperationOutcome {
    OperationOutcome::Failed(OperationFailureView {
        id: operation_target_label(target),
        message,
    })
}

/// 入力順を維持して一括操作レポートへ変換する
fn operation_report_from_outcomes(outcomes: Vec<Option<OperationOutcome>>) -> OperationReport {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for outcome in outcomes.into_iter().flatten() {
        match outcome {
            OperationOutcome::Succeeded(success) => succeeded.push(success),
            OperationOutcome::Failed(failure) => failed.push(failure),
            OperationOutcome::Skipped => {}
        }
    }

    OperationReport { succeeded, failed }
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
    use fwd_deck_core::{
        ConfigSource, TunnelState, TunnelStateFile,
        state::{read_state_file, write_state_file},
    };
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

    /// 未変更の preferences 書き込みがファイル作成を省略することを検証する
    #[test]
    fn unchanged_preferences_write_is_skipped() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("preferences.toml");
        let preferences = AppPreferences::default();

        write_preferences_file_if_changed(&path, &preferences, &preferences)
            .expect("skip unchanged preferences write");

        assert!(!path.exists());
    }

    /// 変更済みの preferences が従来どおり保存されることを検証する
    #[test]
    fn changed_preferences_write_is_persisted() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("preferences.toml");
        let original = AppPreferences::default();
        let next = AppPreferences {
            use_global: false,
            ..AppPreferences::default()
        };

        write_preferences_file_if_changed(&path, &original, &next)
            .expect("write changed preferences");
        let persisted = read_preferences_file(&path).expect("read persisted preferences");

        assert_eq!(persisted, next);
    }

    /// version 1 の preferences が現在の既定値で補完されることを検証する
    #[test]
    fn version_one_preferences_are_normalized_to_current_defaults() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("preferences.toml");
        fs::write(
            &path,
            r#"
version = 1
workspace_history = []
use_global = true
"#,
        )
        .expect("write legacy preferences");
        let mut preferences = read_preferences_file(&path).expect("read legacy preferences");

        normalize_loaded_preferences(&mut preferences);

        assert_eq!(preferences.version, APP_PREFERENCES_VERSION);
        assert!(!preferences.hide_dock_icon_when_window_hidden);
        assert!(!preferences.auto_stop_tunnels_on_quit);
    }

    /// Dock 非表示設定が preferences に保存されることを検証する
    #[test]
    fn dock_visibility_preference_is_persisted() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("preferences.toml");
        let preferences = AppPreferences {
            hide_dock_icon_when_window_hidden: true,
            ..AppPreferences::default()
        };

        write_preferences_file(&path, &preferences).expect("write preferences");
        let persisted = read_preferences_file(&path).expect("read preferences");

        assert!(persisted.hide_dock_icon_when_window_hidden);
    }

    /// 終了時自動停止設定が preferences に保存されることを検証する
    #[test]
    fn auto_stop_tunnels_on_quit_preference_is_persisted() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let path = temp_dir.path().join("preferences.toml");
        let preferences = AppPreferences {
            auto_stop_tunnels_on_quit: true,
            ..AppPreferences::default()
        };

        write_preferences_file(&path, &preferences).expect("write preferences");
        let persisted = read_preferences_file(&path).expect("read preferences");

        assert!(persisted.auto_stop_tunnels_on_quit);
    }

    /// ワークスペース選択入力から Dock 非表示設定が反映されることを検証する
    #[test]
    fn workspace_selection_updates_dock_visibility_preference() {
        let mut preferences = AppPreferences::default();

        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: None,
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: Some(true),
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("apply dock visibility preference");

        assert!(preferences.hide_dock_icon_when_window_hidden);
    }

    /// ワークスペース選択入力から終了時自動停止設定が反映されることを検証する
    #[test]
    fn workspace_selection_updates_auto_stop_tunnels_on_quit_preference() {
        let mut preferences = AppPreferences::default();

        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: None,
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: Some(true),
            },
        )
        .expect("apply quit auto-stop preference");

        assert!(preferences.auto_stop_tunnels_on_quit);
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
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("select first workspace");
        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(second.path().display().to_string()),
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("select second workspace");
        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(first.path().display().to_string()),
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
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
                    hide_dock_icon_when_window_hidden: None,
                    auto_stop_tunnels_on_quit: None,
                },
            )
            .expect("select workspace");
        }

        assert_eq!(preferences.workspace_history.len(), WORKSPACE_HISTORY_LIMIT);
    }

    /// active workspace と同じ履歴行を削除しても active 設定が残ることを検証する
    #[test]
    fn workspace_history_removal_keeps_active_workspace() {
        let active = TempDir::new().expect("create active workspace");
        let other = TempDir::new().expect("create other workspace");
        let active_path = fs::canonicalize(active.path()).expect("canonical active workspace");
        let other_path = fs::canonicalize(other.path()).expect("canonical other workspace");
        let mut preferences = AppPreferences {
            active_workspace_path: Some(active_path.clone()),
            workspace_history: vec![active_path.clone(), other_path.clone()],
            ..AppPreferences::default()
        };
        let workspace_path = active.path().display().to_string();

        remove_workspace_history_entry_from_preferences(&mut preferences, &workspace_path)
            .expect("remove active workspace from history");

        assert_eq!(
            preferences.active_workspace_path.as_deref(),
            Some(active_path.as_path())
        );
        assert_eq!(preferences.workspace_history, vec![other_path]);
    }

    /// 未変更の active workspace 適用で削除済み履歴が復元されないことを検証する
    #[test]
    fn unchanged_workspace_selection_does_not_restore_removed_history_entry() {
        let workspace = TempDir::new().expect("create workspace");
        let workspace_path = workspace.path().display().to_string();
        let canonical_path = fs::canonicalize(workspace.path()).expect("canonical workspace");
        let mut preferences = AppPreferences::default();

        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(workspace_path.clone()),
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("select workspace");
        remove_workspace_history_entry_from_preferences(&mut preferences, &workspace_path)
            .expect("remove workspace from history");
        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(workspace_path),
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("apply unchanged workspace");

        assert_eq!(
            preferences.active_workspace_path.as_deref(),
            Some(canonical_path.as_path())
        );
        assert!(preferences.workspace_history.is_empty());
    }

    /// 別 workspace の選択で履歴が従来どおり更新されることを検証する
    #[test]
    fn changed_workspace_selection_remembers_workspace_after_history_removal() {
        let first = TempDir::new().expect("create first workspace");
        let second = TempDir::new().expect("create second workspace");
        let first_path = first.path().display().to_string();
        let second_path = second.path().display().to_string();
        let canonical_second = fs::canonicalize(second.path()).expect("canonical second workspace");
        let mut preferences = AppPreferences::default();

        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(first_path.clone()),
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("select first workspace");
        remove_workspace_history_entry_from_preferences(&mut preferences, &first_path)
            .expect("remove first workspace from history");
        apply_workspace_selection(
            &mut preferences,
            WorkspaceSelection {
                workspace_path: Some(second_path),
                global_config_path: None,
                use_global: None,
                hide_dock_icon_when_window_hidden: None,
                auto_stop_tunnels_on_quit: None,
            },
        )
        .expect("select second workspace");

        assert_eq!(preferences.workspace_history, vec![canonical_second]);
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
    fn start_tunnel_result_for_app_reports_already_running_tunnel_as_success() {
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

        let result = fwd_deck_core::start_tunnel(&tunnel, &state_path);
        let message = start_tunnel_result_for_app(result).expect("report already running tunnel");

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

    /// ワークスペース切り替えが旧 local state と CLI 互換 state の local 状態を停止することを検証する
    #[test]
    fn workspace_switch_stops_old_workspace_local_state() {
        let workspace = TempDir::new().expect("create workspace");
        let temp_dir = TempDir::new().expect("create state directory");
        let local_config_path = workspace.path().join("fwd-deck.toml");
        let global_state_path = temp_dir.path().join("global-state.toml");
        let workspace_state_path = temp_dir.path().join("workspace-state.toml");
        let mut child = TestChild::sleep();
        fs::write(&local_config_path, "").expect("write local config");
        write_state_file(
            &global_state_path,
            &state_file(vec![tunnel_state(
                "cli-db",
                ConfigSourceKind::Local,
                local_config_path.clone(),
                u32::MAX,
            )]),
        )
        .expect("write global state");
        write_state_file(
            &workspace_state_path,
            &state_file(vec![tunnel_state(
                "app-db",
                ConfigSourceKind::Local,
                local_config_path.clone(),
                child.pid(),
            )]),
        )
        .expect("write workspace state");
        let paths = runtime_paths_for_state_paths(
            Some(local_config_path),
            global_state_path.clone(),
            Some(workspace_state_path.clone()),
        );

        let stopped_count =
            stop_previous_workspace_tunnels(&paths).expect("stop old workspace tunnels");
        child.wait_for_exit();
        let global_state = read_state_file(&global_state_path).expect("read global state");
        let workspace_state = read_state_file(&workspace_state_path).expect("read workspace state");

        assert_eq!(stopped_count, 2);
        assert!(global_state.tunnels.is_empty());
        assert!(workspace_state.tunnels.is_empty());
    }

    /// ワークスペース切り替えが global 設定由来の状態を停止対象から除外することを検証する
    #[test]
    fn workspace_switch_keeps_global_source_state() {
        let workspace = TempDir::new().expect("create workspace");
        let temp_dir = TempDir::new().expect("create state directory");
        let local_config_path = workspace.path().join("fwd-deck.toml");
        let global_config_path = temp_dir.path().join("global-fwd-deck.toml");
        let global_state_path = temp_dir.path().join("global-state.toml");
        fs::write(&local_config_path, "").expect("write local config");
        write_state_file(
            &global_state_path,
            &state_file(vec![tunnel_state(
                "global-db",
                ConfigSourceKind::Global,
                global_config_path,
                u32::MAX,
            )]),
        )
        .expect("write global state");
        let paths = runtime_paths_for_state_paths(Some(local_config_path), global_state_path, None);

        let stopped_count =
            stop_previous_workspace_tunnels(&paths).expect("stop old workspace tunnels");
        let global_state = read_state_file(&paths.global_state_path).expect("read global state");

        assert_eq!(stopped_count, 0);
        assert_eq!(global_state.tunnels.len(), 1);
        assert_eq!(global_state.tunnels[0].id, "global-db");
    }

    /// 停止失敗時に新しいワークスペース設定が保存されないことを検証する
    #[test]
    fn workspace_switch_stop_failure_keeps_previous_preferences() {
        let app_config_dir = TempDir::new().expect("create app config directory");
        let previous_workspace = TempDir::new().expect("create previous workspace");
        let next_workspace = TempDir::new().expect("create next workspace");
        let preferences_path = app_config_dir.path().join("preferences.toml");
        let previous_workspace_path =
            fs::canonicalize(previous_workspace.path()).expect("canonical previous workspace");
        let preferences = AppPreferences {
            active_workspace_path: Some(previous_workspace_path.clone()),
            workspace_history: vec![previous_workspace_path],
            ..AppPreferences::default()
        };
        write_preferences_file(&preferences_path, &preferences).expect("write preferences");
        let selection =
            workspace_selection_for_path(next_workspace.path()).expect("build workspace selection");

        let result = apply_workspace_switch_with_stop(
            app_config_dir.path(),
            &preferences_path,
            selection,
            |_paths| {
                Err(AppError::WorkspaceSwitchStop {
                    id: "workspace:db".to_owned(),
                    message: "stop failed".to_owned(),
                })
            },
        );
        let persisted = read_preferences_file(&preferences_path).expect("read preferences");

        assert!(result.is_err());
        assert_eq!(persisted, preferences);
    }

    /// スキップされた操作対象が成功件数と失敗件数に含まれないことを検証する
    #[test]
    fn run_tunnel_operations_omits_skipped_targets() {
        let targets = vec![OperationTargetInput {
            id: "missing".to_owned(),
            runtime_scope: None,
        }];

        let report = run_tunnel_operations_with_progress(
            &targets,
            |_target| Ok::<Option<String>, AppError>(None),
            |_target| {},
        )
        .expect("run operation");

        assert!(report.succeeded.is_empty());
        assert!(report.failed.is_empty());
    }

    /// start / stop 操作が排他ロック内で実行されることを検証する
    #[test]
    fn operation_lock_runs_operation_while_locked() {
        let operation_lock = OperationLockState::default();
        let mut operation_ran = false;

        with_operation_lock(&operation_lock, || {
            assert!(operation_lock.0.try_lock().is_err());
            operation_ran = true;
            Ok::<(), AppError>(())
        })
        .expect("run operation with lock");

        assert!(operation_ran);
    }

    /// トレイメニュー項目が runtime 状態に応じた toggle 操作を表現することを検証する
    #[test]
    fn tray_menu_items_reflect_running_and_stale_tunnels() {
        let running =
            resolved_tunnel_with_port("running-db", PathBuf::from("fwd-deck.toml"), 15432);
        let stale = resolved_tunnel_with_port("stale-db", PathBuf::from("fwd-deck.toml"), 15433);
        let idle = resolved_tunnel_with_port("idle-db", PathBuf::from("fwd-deck.toml"), 15434);
        let config = EffectiveConfig::new(
            Vec::new(),
            vec![running.clone(), stale.clone(), idle.clone()],
        );
        let statuses = vec![
            scoped_runtime_status(&running, RuntimeScope::Workspace, ProcessState::Running),
            scoped_runtime_status(&stale, RuntimeScope::Workspace, ProcessState::Stale),
        ];
        let validation = validate_config(&config);

        let items = tray_tunnel_menu_items(&config, &statuses, &validation);

        let running_item = tray_item_by_id(&items, "running-db");
        assert!(running_item.checked);
        assert!(running_item.enabled);
        assert_eq!(running_item.action.operation, TrayTunnelOperation::Stop);
        assert_eq!(
            running_item.action.runtime_scope,
            Some(RuntimeScope::Workspace)
        );

        let stale_item = tray_item_by_id(&items, "stale-db");
        assert_eq!(stale_item.label, "stale-db (stale)");
        assert!(!stale_item.checked);
        assert!(stale_item.enabled);
        assert_eq!(stale_item.action.operation, TrayTunnelOperation::Start);

        let idle_item = tray_item_by_id(&items, "idle-db");
        assert!(!idle_item.checked);
        assert!(idle_item.enabled);
        assert_eq!(idle_item.action.operation, TrayTunnelOperation::Start);
    }

    /// トレイアイコン種別が起動中トンネルの有無に追従することを検証する
    #[test]
    fn tray_icon_kind_reflects_running_tunnels() {
        let running =
            resolved_tunnel_with_port("running-db", PathBuf::from("fwd-deck.toml"), 15432);
        let stale = resolved_tunnel_with_port("stale-db", PathBuf::from("fwd-deck.toml"), 15433);
        let stale_status =
            scoped_runtime_status(&stale, RuntimeScope::Workspace, ProcessState::Stale);
        let running_status =
            scoped_runtime_status(&running, RuntimeScope::Workspace, ProcessState::Running);

        assert_eq!(tray_icon_kind(&[]), TrayIconKind::Idle);
        assert_eq!(tray_icon_kind(&[stale_status]), TrayIconKind::Idle);
        assert_eq!(tray_icon_kind(&[running_status]), TrayIconKind::Active);
    }

    /// 設定エラー時は開始系のトレイ項目だけが無効化されることを検証する
    #[test]
    fn tray_menu_items_disable_start_actions_when_config_is_invalid() {
        let running =
            resolved_tunnel_with_port("running-db", PathBuf::from("fwd-deck.toml"), 15432);
        let mut invalid =
            resolved_tunnel_with_port("idle-db", PathBuf::from("fwd-deck.toml"), 15433);
        invalid.tunnel.remote_host.clear();
        let config = EffectiveConfig::new(Vec::new(), vec![running.clone(), invalid.clone()]);
        let statuses = vec![scoped_runtime_status(
            &running,
            RuntimeScope::Workspace,
            ProcessState::Running,
        )];
        let validation = validate_config(&config);

        let items = tray_tunnel_menu_items(&config, &statuses, &validation);

        assert!(tray_item_by_id(&items, "running-db").enabled);
        assert!(!tray_item_by_id(&items, "idle-db").enabled);
    }

    /// トレイのワークスペース項目が現在値と履歴を分けて表現することを検証する
    #[test]
    fn tray_workspace_menu_items_reflect_current_and_history() {
        let active = PathBuf::from("/tmp/fwd-deck-active");
        let recent = PathBuf::from("/tmp/fwd-deck-recent");
        let preferences = AppPreferences {
            active_workspace_path: Some(active.clone()),
            workspace_history: vec![active.clone(), recent.clone()],
            ..AppPreferences::default()
        };

        let items = tray_workspace_menu_items(&preferences);

        let current = &items[0];
        assert_eq!(current.menu_id, TRAY_MENU_CURRENT_WORKSPACE);
        assert!(current.label.contains(&display_path(&active)));
        assert!(current.checked);
        assert!(!current.enabled);
        assert!(current.action.is_none());

        let recent_item = workspace_item_by_path(&items, &recent);
        assert_eq!(recent_item.label, display_path(&recent));
        assert!(!recent_item.checked);
        assert!(recent_item.enabled);
    }

    /// 終了時対象収集が global state の起動中トンネルを含めることを検証する
    #[test]
    fn collect_visible_quit_targets_includes_running_global_state() {
        let temp_dir = TempDir::new().expect("create state directory");
        let global_state_path = temp_dir.path().join("global-state.toml");
        let global_config_path = temp_dir.path().join("global-fwd-deck.toml");
        write_state_file(
            &global_state_path,
            &state_file(vec![tunnel_state(
                "db",
                ConfigSourceKind::Global,
                global_config_path,
                std::process::id(),
            )]),
        )
        .expect("write global state");
        let paths = runtime_paths_for_state_paths(None, global_state_path.clone(), None);

        let targets = collect_visible_quit_tunnel_targets(&paths).expect("collect quit targets");

        assert_eq!(targets.running.len(), 1);
        assert!(targets.stale.is_empty());
        assert_eq!(targets.running[0].id, "db");
        assert_eq!(targets.running[0].runtime_scope, RuntimeScope::Global);
        assert_eq!(targets.running[0].state_path, global_state_path);
        assert_eq!(targets.running[0].process_state, ProcessState::Running);
    }

    /// 終了時対象収集が stale state を確認対象から分離することを検証する
    #[test]
    fn collect_visible_quit_targets_separates_stale_state() {
        let temp_dir = TempDir::new().expect("create state directory");
        let global_state_path = temp_dir.path().join("global-state.toml");
        let global_config_path = temp_dir.path().join("global-fwd-deck.toml");
        write_state_file(
            &global_state_path,
            &state_file(vec![tunnel_state(
                "db",
                ConfigSourceKind::Global,
                global_config_path,
                u32::MAX,
            )]),
        )
        .expect("write global state");
        let paths = runtime_paths_for_state_paths(None, global_state_path, None);

        let targets = collect_visible_quit_tunnel_targets(&paths).expect("collect quit targets");

        assert!(targets.running.is_empty());
        assert_eq!(targets.stale.len(), 1);
        assert_eq!(targets.stale[0].process_state, ProcessState::Stale);
    }

    /// 終了時対象収集が別ワークスペース由来の state を除外することを検証する
    #[test]
    fn collect_visible_quit_targets_excludes_other_workspace_state() {
        let active_workspace = TempDir::new().expect("create active workspace");
        let other_workspace = TempDir::new().expect("create other workspace");
        let temp_dir = TempDir::new().expect("create state directory");
        let active_config_path = active_workspace.path().join("fwd-deck.toml");
        let other_config_path = other_workspace.path().join("fwd-deck.toml");
        let global_state_path = temp_dir.path().join("global-state.toml");
        fs::write(&active_config_path, "").expect("write active config");
        fs::write(&other_config_path, "").expect("write other config");
        write_state_file(
            &global_state_path,
            &state_file(vec![tunnel_state(
                "db",
                ConfigSourceKind::Local,
                other_config_path,
                std::process::id(),
            )]),
        )
        .expect("write global state");
        let paths =
            runtime_paths_for_state_paths(Some(active_config_path), global_state_path, None);

        let targets = collect_visible_quit_tunnel_targets(&paths).expect("collect quit targets");

        assert!(targets.running.is_empty());
        assert!(targets.stale.is_empty());
    }

    /// 終了時対象収集が workspace state のパスを保持することを検証する
    #[test]
    fn collect_visible_quit_targets_uses_workspace_state_path() {
        let workspace = TempDir::new().expect("create workspace");
        let temp_dir = TempDir::new().expect("create state directory");
        let local_config_path = workspace.path().join("fwd-deck.toml");
        let global_state_path = temp_dir.path().join("global-state.toml");
        let workspace_state_path = temp_dir.path().join("workspace-state.toml");
        fs::write(&local_config_path, "").expect("write local config");
        write_state_file(
            &workspace_state_path,
            &state_file(vec![tunnel_state(
                "db",
                ConfigSourceKind::Local,
                local_config_path.clone(),
                std::process::id(),
            )]),
        )
        .expect("write workspace state");
        let paths = runtime_paths_for_state_paths(
            Some(local_config_path),
            global_state_path,
            Some(workspace_state_path.clone()),
        );

        let targets = collect_visible_quit_tunnel_targets(&paths).expect("collect quit targets");

        assert_eq!(targets.running.len(), 1);
        assert_eq!(targets.running[0].runtime_scope, RuntimeScope::Workspace);
        assert_eq!(targets.running[0].state_path, workspace_state_path);
    }

    /// stale state が確認なしの掃除対象として削除されることを検証する
    #[test]
    fn stop_quit_tunnel_targets_removes_stale_state() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");
        write_state_file(
            &state_path,
            &state_file(vec![tunnel_state(
                "db",
                ConfigSourceKind::Global,
                temp_dir.path().join("fwd-deck.toml"),
                u32::MAX,
            )]),
        )
        .expect("write state");
        let target = QuitTunnelTarget {
            id: "db".to_owned(),
            runtime_scope: RuntimeScope::Global,
            state_path: state_path.clone(),
            process_state: ProcessState::Stale,
        };

        stop_quit_tunnel_targets(&[target]).expect("remove stale state");
        let state = read_state_file(&state_path).expect("read state");

        assert!(state.tunnels.is_empty());
    }

    /// 終了確認メッセージがダイアログ内で折り返されにくい短い文言であることを検証する
    #[test]
    fn quit_confirmation_message_uses_compact_text() {
        let message = quit_confirmation_message();

        assert_eq!(
            message,
            "起動中のポートフォワーディングがあります。\n停止して終了しますか？"
        );
    }

    /// 終了確認ダイアログのカスタムボタン結果を内部アクションへ変換できることを検証する
    #[test]
    fn quit_dialog_action_maps_custom_buttons() {
        assert_eq!(
            quit_dialog_action(&MessageDialogResult::Custom(
                QUIT_DIALOG_STOP_LABEL.to_owned()
            )),
            QuitDialogAction::StopAndQuit
        );
        assert_eq!(
            quit_dialog_action(&MessageDialogResult::Custom(
                QUIT_DIALOG_KEEP_LABEL.to_owned()
            )),
            QuitDialogAction::QuitOnly
        );
        assert_eq!(
            quit_dialog_action(&MessageDialogResult::Cancel),
            QuitDialogAction::Cancel
        );
    }

    /// 終了時自動停止設定が起動中トンネルの扱いを切り替えることを検証する
    #[test]
    fn quit_running_tunnels_action_reflects_auto_stop_preference() {
        let prompt_preferences = AppPreferences::default();
        let auto_stop_preferences = AppPreferences {
            auto_stop_tunnels_on_quit: true,
            ..AppPreferences::default()
        };

        assert_eq!(
            quit_running_tunnels_action(&prompt_preferences),
            QuitRunningTunnelsAction::Prompt
        );
        assert_eq!(
            quit_running_tunnels_action(&auto_stop_preferences),
            QuitRunningTunnelsAction::AutoStop
        );
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

    /// テスト用の runtime paths を state path 指定で生成する
    fn runtime_paths_for_state_paths(
        local_config_path: Option<PathBuf>,
        global_state_path: PathBuf,
        workspace_state_path: Option<PathBuf>,
    ) -> RuntimePaths {
        RuntimePaths {
            preferences: AppPreferences::default(),
            config_paths: ConfigPaths::new(
                None,
                local_config_path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("fwd-deck.toml")),
            ),
            local_config_path,
            global_config_display_path: None,
            global_state_path,
            workspace_state_path,
        }
    }

    /// テスト用の状態ファイルを生成する
    fn state_file(tunnels: Vec<TunnelState>) -> TunnelStateFile {
        TunnelStateFile { tunnels }
    }

    /// テスト用の tunnel state を生成する
    fn tunnel_state(
        id: &str,
        source_kind: ConfigSourceKind,
        source_path: PathBuf,
        pid: u32,
    ) -> TunnelState {
        TunnelState {
            id: id.to_owned(),
            pid,
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
        }
    }

    /// テスト用の runtime status を生成する
    fn runtime_status(source_kind: ConfigSourceKind, source_path: PathBuf) -> TunnelRuntimeStatus {
        TunnelRuntimeStatus {
            state: tunnel_state("db", source_kind, source_path, 1000),
            process_state: ProcessState::Running,
        }
    }

    /// テスト用の scoped runtime status を生成する
    fn scoped_runtime_status(
        resolved: &ResolvedTunnelConfig,
        runtime_scope: RuntimeScope,
        process_state: ProcessState,
    ) -> ScopedRuntimeStatus {
        ScopedRuntimeStatus {
            runtime_scope,
            status: TunnelRuntimeStatus {
                state: TunnelState::from_resolved_tunnel(resolved, 1000, 1_700_000_000),
                process_state,
            },
        }
    }

    /// テスト用のトレイ項目を ID で取得する
    fn tray_item_by_id<'a>(items: &'a [TrayTunnelMenuItem], id: &str) -> &'a TrayTunnelMenuItem {
        items
            .iter()
            .find(|item| item.action.id == id)
            .expect("tray item should exist")
    }

    /// テスト用のワークスペース項目を path で取得する
    fn workspace_item_by_path<'a>(
        items: &'a [TrayWorkspaceMenuItem],
        path: &Path,
    ) -> &'a TrayWorkspaceMenuItem {
        items
            .iter()
            .find(|item| {
                item.action
                    .as_ref()
                    .map(|action| action.workspace_path.as_path() == path)
                    .unwrap_or(false)
            })
            .expect("workspace item should exist")
    }

    /// テスト用の resolved tunnel を生成する
    fn resolved_tunnel(id: &str, source_path: PathBuf) -> ResolvedTunnelConfig {
        resolved_tunnel_with_port(id, source_path, 15432)
    }

    /// テスト用の local port 指定 resolved tunnel を生成する
    fn resolved_tunnel_with_port(
        id: &str,
        source_path: PathBuf,
        local_port: u16,
    ) -> ResolvedTunnelConfig {
        ResolvedTunnelConfig::new(
            ConfigSource::new(ConfigSourceKind::Local, source_path),
            TunnelConfig {
                id: id.to_owned(),
                description: None,
                tags: Vec::new(),
                local_host: None,
                local_port,
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

    /// テスト用の短命な子プロセスを保持する
    struct TestChild {
        child: Option<std::process::Child>,
    }

    impl TestChild {
        /// sleep プロセスを起動する
        fn sleep() -> Self {
            let child = std::process::Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("spawn sleep process");

            Self { child: Some(child) }
        }

        /// 子プロセスの PID を取得する
        fn pid(&self) -> u32 {
            self.child.as_ref().expect("child should be running").id()
        }

        /// 子プロセスの終了を待機する
        fn wait_for_exit(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.wait();
            }
        }
    }

    impl Drop for TestChild {
        /// 残存プロセスを終了する
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
