use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::{self, Display},
    fs,
    io::{self, IsTerminal},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use fwd_deck_core::{
    ConfigEditError, ConfigPaths, ConfigSourceKind, DEFAULT_LOCAL_HOST, EffectiveConfig,
    ProcessState, ResolvedTimeoutConfig, ResolvedTunnelConfig, StartedTunnel, StoppedTunnel,
    TimeoutConfig, TunnelConfig, TunnelRuntimeError, TunnelRuntimeStatus, TunnelState,
    ValidationReport, add_tunnel_to_config_file, build_ssh_command_args,
    default_global_config_path, default_local_config_path, default_state_file_path,
    filter_tunnels_by_tags, format_path_for_display, load_effective_config, normalize_tag,
    read_config_file, remove_tunnel_from_config_file, runtime_id_for_resolved_tunnel, start_tunnel,
    start_tunnels, stop_tunnel, tag_is_valid, tunnel_statuses, update_tunnel_in_config_file,
    validate_config,
};
use inquire::{Confirm, InquireError, MultiSelect, Select, Text};
use serde::Serialize;
use thiserror::Error;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_WATCH_INTERVAL_SECONDS: u64 = 5;
const LIST_ID_MIN_WIDTH: usize = 24;
const LIST_LOCAL_MIN_WIDTH: usize = 24;
const LIST_REMOTE_MIN_WIDTH: usize = 32;
const LIST_REMOTE_HOST_MAX_WIDTH: usize = 40;
const LIST_SSH_MIN_WIDTH: usize = 32;
const LIST_TAGS_MIN_WIDTH: usize = 24;
const STATUS_ID_MIN_WIDTH: usize = 24;
const STATUS_LOCAL_MIN_WIDTH: usize = 24;
const STATUS_REMOTE_MIN_WIDTH: usize = 32;
const STATUS_PID_MIN_WIDTH: usize = 8;
const STATUS_STATE_MIN_WIDTH: usize = 10;
const TRUNCATION_MARKER: &str = "...";
#[cfg(any(test, target_os = "macos"))]
const FWD_DECK_APP_BUNDLE_IDENTIFIER: &str = "dev.oiekjr.fwddeck";
#[cfg(any(test, target_os = "macos"))]
const FWD_DECK_APP_OPEN_WORKSPACE_ARG: &str = "--fwd-deck-open-workspace";
const CLI_AFTER_HELP: &str = "\
例:
  fwd-deck validate
  fwd-deck doctor
  fwd-deck list --tag dev
  fwd-deck start dev-db --dry-run
  fwd-deck open
  fwd-deck status

設定:
  既定では ./fwd-deck.toml と ~/.config/fwd-deck/config.toml を読み込みます。
  起動中トンネルの状態は ~/.local/state/fwd-deck/state.toml に保存します。";
const OPEN_AFTER_HELP: &str = "\
例:
  fwd-deck open
  fwd-deck open ~/projects/my-service

補足:
  PATH を省略すると、現在のディレクトリを Workspace として macOSアプリを開きます。
  既存アプリが起動中の場合は、既存ウィンドウで Workspace を切り替えます。
  Workspace 切り替え時は、旧 Workspace の local トンネルを停止します。";
const LIST_AFTER_HELP: &str = "\
例:
  fwd-deck list
  fwd-deck --json list
  fwd-deck list --wide
  fwd-deck list --tag dev --tag project-a
  fwd-deck list --query db

補足:
  --tag は複数指定でき、指定したタグをすべて持つトンネルだけを表示します。
  --query は NAME と description を大文字小文字を区別せずに検索します。
  --wide は REMOTE の host 部分を省略せずに表示します。";
const SHOW_AFTER_HELP: &str = "\
例:
  fwd-deck show dev-db
  fwd-deck --json show dev-db

補足:
  統合後の設定、接続先、有効なタイムアウト、読み込み元の設定ファイルを表示します。";
const START_AFTER_HELP: &str = "\
例:
  fwd-deck start
  fwd-deck start dev-db
  fwd-deck start --all
  fwd-deck start --all --parallel 4
  fwd-deck start --tag dev --tag project-a
  fwd-deck start dev-db --dry-run
  fwd-deck --json start dev-db --dry-run

補足:
  NAME を省略すると対話選択を表示します。
  --all、NAME、--tag は同時に指定できません。
  --parallel は複数トンネルの開始処理を指定件数まで並列実行します。
  --dry-run は SSH を起動せず、状態ファイルも更新しません。";
const RECOVER_AFTER_HELP: &str = "\
例:
  fwd-deck recover
  fwd-deck recover dev-db

補足:
  NAME を省略すると、状態ファイルで stale と判定された追跡中トンネルを再起動します。";
const WATCH_AFTER_HELP: &str = "\
例:
  fwd-deck watch
  fwd-deck watch dev-db --interval-seconds 5

補足:
  状態ファイル上の追跡中トンネルを監視し、stale になった場合に現在の設定で再起動します。";
const STATUS_AFTER_HELP: &str = "\
例:
  fwd-deck status
  fwd-deck --json status

補足:
  状態ファイルに記録された PID を使い、追跡中トンネルが実行中か stale かを表示します。";
const STOP_AFTER_HELP: &str = "\
例:
  fwd-deck stop
  fwd-deck stop dev-db
  fwd-deck stop --all
  fwd-deck stop dev-db --dry-run

補足:
  NAME を省略すると対話選択を表示します。
  --all と NAME は同時に指定できません。
  --dry-run はプロセスを停止せず、状態ファイルも更新しません。";
const CONFIG_AFTER_HELP: &str = "\
例:
  fwd-deck config add
  fwd-deck config edit dev-db
  fwd-deck config remove --scope local

補足:
  --scope を省略すると、編集する local または global 設定を対話選択します。";
const CONFIG_ADD_AFTER_HELP: &str = "\
例:
  fwd-deck config add
  fwd-deck config add --scope local
  fwd-deck config add --scope global

  補足:
  --scope を省略すると、編集する local または global 設定を対話選択します。
  local は ./fwd-deck.toml、global は ~/.config/fwd-deck/config.toml を対象にします。";
const CONFIG_EDIT_AFTER_HELP: &str = "\
例:
  fwd-deck config edit dev-db
  fwd-deck config edit dev-db --scope local
  fwd-deck config edit dev-db --scope global

補足:
  既存値を初期値として表示し、空入力は既存値維持として扱います。
  同じ NAME が local と global の両方に存在する場合、対話実行時は編集対象を選択します。
  非対話実行時は --scope を指定します。";
const CONFIG_REMOVE_AFTER_HELP: &str = "\
例:
  fwd-deck config remove
  fwd-deck config remove --scope local
  fwd-deck config remove --scope global

補足:
  --scope を省略すると、編集する local または global 設定を対話選択します。
  選択した設定ファイルに定義されているトンネルだけを削除対象にします。";
const COMPLETION_AFTER_HELP: &str = "\
例:
  fwd-deck completion zsh
  fwd-deck completion zsh > ~/.zfunc/_fwd-deck

補足:
  対応シェルは bash、elvish、fish、powershell、zsh です。
  zsh では出力先を fpath に追加し、compinit を有効にします。";
const VALIDATE_AFTER_HELP: &str = "\
例:
  fwd-deck validate
  fwd-deck --json validate
  fwd-deck --config ./my-fwd-deck.toml validate

補足:
  読み込んだ local と global の設定を統合し、エラーと warning を表示します。";
const DOCTOR_AFTER_HELP: &str = "\
例:
  fwd-deck doctor

補足:
  設定ファイルの有無、設定検証、状態ファイルの読み書き、ssh / lsof の起動可否、identity_file の存在、local endpoint の使用状況を確認します。";

/// fwd-deck の CLI 引数を表現する
#[derive(Debug, Parser)]
#[command(
    name = "fwd-deck",
    version,
    about = "設定ファイルに定義したポートフォワーディングを操作する",
    after_help = CLI_AFTER_HELP
)]
struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "local設定ファイルを PATH から読み込む"
    )]
    config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "global設定ファイルを PATH から読み込む"
    )]
    global_config: Option<PathBuf>,

    #[arg(long, global = true, help = "global設定ファイルを読み込まない")]
    no_global: bool,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "実行状態ファイルを PATH から読み書きする"
    )]
    state: Option<PathBuf>,

    #[arg(long, global = true, help = "Print supported command output as JSON")]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

/// fwd-deck が提供するサブコマンドを表現する
#[derive(Debug, Clone, Subcommand)]
enum Command {
    #[command(
        about = "現在または指定ディレクトリを Workspace として macOSアプリを開く",
        after_help = OPEN_AFTER_HELP
    )]
    Open {
        #[arg(value_name = "PATH", help = "Workspace として開くディレクトリ")]
        path: Option<PathBuf>,
    },
    #[command(about = "設定済みトンネルを一覧表示する", after_help = LIST_AFTER_HELP)]
    List {
        #[arg(long = "tag", value_name = "TAG", help = "指定タグで絞り込む")]
        tags: Vec<String>,
        #[arg(
            long,
            value_name = "TEXT",
            help = "NAME または description の部分一致で絞り込む"
        )]
        query: Option<String>,
        #[arg(long, help = "REMOTE の host 部分を省略せずに表示する")]
        wide: bool,
    },
    #[command(about = "設定済みトンネルの詳細を表示する", after_help = SHOW_AFTER_HELP)]
    Show {
        #[arg(value_name = "NAME", help = "詳細を表示するトンネル名")]
        name: String,
        #[arg(long, value_enum, help = "対象スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(about = "設定済みトンネルを起動する", after_help = START_AFTER_HELP)]
    Start {
        #[arg(value_name = "NAME", help = "起動するトンネル名")]
        names: Vec<String>,
        #[arg(
            long = "tag",
            value_name = "TAG",
            help = "指定タグに一致するトンネルを起動する"
        )]
        tags: Vec<String>,
        #[arg(long, help = "設定済みの全トンネルを起動する")]
        all: bool,
        #[arg(
            long,
            default_value_t = 1,
            help = "複数トンネルの開始処理を指定件数まで並列実行する"
        )]
        parallel: usize,
        #[arg(long, help = "SSH を起動せずに実行予定だけを表示する")]
        dry_run: bool,
        #[arg(long, value_enum, help = "対象スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(about = "stale な追跡中トンネルを再起動する", after_help = RECOVER_AFTER_HELP)]
    Recover {
        #[arg(value_name = "NAME", help = "再起動するトンネル名")]
        names: Vec<String>,
        #[arg(long, value_enum, help = "対象スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(about = "追跡中トンネルを監視して stale 時に再起動する", after_help = WATCH_AFTER_HELP)]
    Watch {
        #[arg(value_name = "NAME", help = "監視するトンネル名")]
        names: Vec<String>,
        #[arg(
            long,
            default_value_t = DEFAULT_WATCH_INTERVAL_SECONDS,
            value_parser = clap::value_parser!(u64).range(1..),
            help = "監視間隔を秒単位で指定する"
        )]
        interval_seconds: u64,
        #[arg(long, value_enum, help = "対象スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(about = "追跡中トンネルの状態を表示する", after_help = STATUS_AFTER_HELP)]
    Status,
    #[command(about = "追跡中トンネルを停止する", after_help = STOP_AFTER_HELP)]
    Stop {
        #[arg(value_name = "NAME", help = "停止するトンネル名")]
        names: Vec<String>,
        #[arg(long, help = "追跡中の全トンネルを停止する")]
        all: bool,
        #[arg(long, help = "プロセスを停止せずに実行予定だけを表示する")]
        dry_run: bool,
        #[arg(long, value_enum, help = "対象スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(about = "設定ファイルを対話形式で編集する", after_help = CONFIG_AFTER_HELP)]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "シェル補完スクリプトを生成する", after_help = COMPLETION_AFTER_HELP)]
    Completion {
        #[arg(value_enum, help = "補完スクリプトを生成するシェル")]
        shell: Shell,
    },
    #[command(about = "設定と実行環境を診断する", after_help = DOCTOR_AFTER_HELP)]
    Doctor,
    #[command(about = "設定ファイルを検証する", after_help = VALIDATE_AFTER_HELP)]
    Validate,
}

/// 設定編集サブコマンドを表現する
#[derive(Debug, Clone, Subcommand)]
enum ConfigCommand {
    #[command(about = "設定ファイルへトンネルを追加する", after_help = CONFIG_ADD_AFTER_HELP)]
    Add {
        #[arg(long, value_enum, help = "編集する設定スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(
        about = "設定ファイルからトンネルを削除する",
        after_help = CONFIG_REMOVE_AFTER_HELP
    )]
    Remove {
        #[arg(long, value_enum, help = "編集する設定スコープ")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(
        about = "設定ファイル内の既存トンネルを編集する",
        after_help = CONFIG_EDIT_AFTER_HELP
    )]
    Edit {
        #[arg(value_name = "NAME", help = "編集するトンネル名")]
        id: String,
        #[arg(long, value_enum, help = "編集する設定スコープ")]
        scope: Option<ConfigScopeArg>,
    },
}

/// CLI で指定する設定スコープを表現する
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ConfigScopeArg {
    Global,
    Local,
}

impl From<ConfigScopeArg> for ConfigSourceKind {
    /// CLI の設定スコープを中核機能の設定種別へ変換する
    fn from(scope: ConfigScopeArg) -> Self {
        match scope {
            ConfigScopeArg::Global => Self::Global,
            ConfigScopeArg::Local => Self::Local,
        }
    }
}

/// CLI で指定されたスコープを任意の設定種別へ変換する
fn scope_kind(scope: Option<ConfigScopeArg>) -> Option<ConfigSourceKind> {
    scope.map(ConfigSourceKind::from)
}

/// CLI 実行時の失敗理由を表現する
#[derive(Debug, Error)]
enum CliError {
    #[error("Failed to get the current directory: {0}")]
    CurrentDir(std::io::Error),
    #[error("Failed to resolve the default global configuration path because HOME is not set")]
    MissingGlobalConfigPath,
    #[error("Failed to resolve the default state file path because HOME is not set")]
    MissingStateHome,
    #[error(
        "Invalid tag: {tag}. Tags may contain only lowercase ASCII letters, numbers, '-', '_', '.', or '/'"
    )]
    InvalidTag { tag: String },
    #[error(
        "Tunnel name exists in multiple configuration files: {id}. Specify --scope in non-interactive mode"
    )]
    AmbiguousConfigEdit { id: String },
    #[error("Workspace directory was not found: {}", format_path_for_display(.path))]
    WorkspaceNotFound { path: PathBuf },
    #[error("Workspace path is not a directory: {}", format_path_for_display(.path))]
    WorkspaceNotDirectory { path: PathBuf },
    #[error("Workspace path must be valid UTF-8: {}", format_path_for_display(.path))]
    WorkspacePathNonUtf8 { path: PathBuf },
    #[cfg(not(target_os = "macos"))]
    #[error("fwd-deck-app is supported only on macOS")]
    AppOpenUnsupported,
    #[cfg(any(test, target_os = "macos"))]
    #[error(
        "Fwd Deck.app が見つかりません。\nmacOSアプリをインストールしてから再実行してください。\n\n  brew install --cask oiekjr/tap/fwd-deck-app"
    )]
    AppNotInstalled,
    #[cfg(any(test, target_os = "macos"))]
    #[error("Fwd Deck.app を起動できませんでした。\n{message}")]
    AppLaunchFailed { message: String },
    #[error("Failed to launch Fwd Deck.app: {0}")]
    AppLaunchIo(#[from] io::Error),
    #[error(transparent)]
    Config(#[from] fwd_deck_core::ConfigLoadError),
    #[error(transparent)]
    ConfigEdit(#[from] ConfigEditError),
    #[error(transparent)]
    Runtime(#[from] TunnelRuntimeError),
    #[error(transparent)]
    Prompt(#[from] InquireError),
    #[error("--json is not supported for this command")]
    JsonUnsupported,
    #[error("Failed to serialize JSON output: {0}")]
    Json(#[from] serde_json::Error),
}

/// CLI の実行入口を初期化する
pub fn run_from_env() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{}", red(&format!("Error: {error}"), OutputStream::Stderr));
            ExitCode::FAILURE
        }
    }
}

/// CLI の処理を実行する
fn run() -> Result<ExitCode, CliError> {
    let cli = Cli::parse();
    let state_path = cli.state.clone();

    match &cli.command {
        Command::Open { path } => {
            reject_json_if_requested(cli.json)?;
            open_app_command(path.clone())
        }
        Command::List { tags, query, wide } => {
            let config = load_config(&cli)?;
            list_command(&config, tags.clone(), query.clone(), *wide, cli.json)
        }
        Command::Show { name, scope } => {
            let config = load_config(&cli)?;
            show_command(&config, name, *scope, cli.json)
        }
        Command::Start {
            names,
            tags,
            all,
            parallel,
            dry_run,
            scope,
        } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            start_command(
                &config,
                &state_path,
                StartCommandOptions {
                    names: names.clone(),
                    tags: tags.clone(),
                    all: *all,
                    parallel: *parallel,
                    dry_run: *dry_run,
                    scope: *scope,
                    json: cli.json,
                },
            )
        }
        Command::Recover { names, scope } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            recover_command(&config, &state_path, names.clone(), *scope)
        }
        Command::Watch {
            names,
            interval_seconds,
            scope,
        } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            watch_command(
                &config,
                &state_path,
                names.clone(),
                *scope,
                *interval_seconds,
            )
        }
        Command::Status => {
            let state_path = resolve_state_path(state_path)?;
            status_command(&state_path, cli.json)
        }
        Command::Stop {
            names,
            all,
            dry_run,
            scope,
        } => {
            reject_json_if_requested(cli.json)?;
            let state_path = resolve_state_path(state_path)?;
            stop_command(&state_path, names.clone(), *all, *dry_run, *scope)
        }
        Command::Config { command } => {
            reject_json_if_requested(cli.json)?;
            let paths = resolve_config_paths(&cli)?;
            config_command(&paths, command.clone())
        }
        Command::Completion { shell } => {
            reject_json_if_requested(cli.json)?;
            completion_command(*shell)
        }
        Command::Doctor => {
            reject_json_if_requested(cli.json)?;
            let config = load_config(&cli)?;
            let paths = resolve_config_paths(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            doctor_command(&config, &paths, &state_path)
        }
        Command::Validate => {
            let config = load_config(&cli)?;
            validate_command(&config, cli.json)
        }
    }
}

/// JSON 非対応コマンドで JSON 出力指定を拒否する
fn reject_json_if_requested(json: bool) -> Result<(), CliError> {
    if json {
        return Err(CliError::JsonUnsupported);
    }

    Ok(())
}

/// CLI 引数に従って設定を読み込む
fn load_config(cli: &Cli) -> Result<EffectiveConfig, CliError> {
    let paths = resolve_config_paths(cli)?;
    load_effective_config(&paths).map_err(CliError::Config)
}

/// CLI 引数から設定ファイルの読込先を解決する
fn resolve_config_paths(cli: &Cli) -> Result<ConfigPaths, CliError> {
    let current_dir = env::current_dir().map_err(CliError::CurrentDir)?;
    let local = cli
        .config
        .clone()
        .unwrap_or_else(|| default_local_config_path(&current_dir));
    let global = if cli.no_global {
        None
    } else {
        cli.global_config
            .clone()
            .or_else(default_global_config_path)
    };

    Ok(ConfigPaths::new(global, local))
}

/// CLI 引数から状態ファイルの保存先を解決する
fn resolve_state_path(state_path: Option<PathBuf>) -> Result<PathBuf, CliError> {
    state_path
        .or_else(default_state_file_path)
        .ok_or(CliError::MissingStateHome)
}

/// シェル補完スクリプトを生成する
fn completion_command(shell: Shell) -> Result<ExitCode, CliError> {
    let mut command = Cli::command();
    let binary_name = command.get_name().to_owned();

    clap_complete::generate(shell, &mut command, binary_name, &mut io::stdout());

    Ok(ExitCode::SUCCESS)
}

/// 現在または指定ディレクトリを Workspace としてアプリを開く
fn open_app_command(path: Option<PathBuf>) -> Result<ExitCode, CliError> {
    let workspace_path = resolve_open_workspace_path(path)?;

    launch_app_for_workspace(&workspace_path)?;
    println!(
        "Opening Fwd Deck workspace: {}",
        format_path_for_display(&workspace_path)
    );

    Ok(ExitCode::SUCCESS)
}

/// アプリへ渡す Workspace パスを絶対パスへ解決する
fn resolve_open_workspace_path(path: Option<PathBuf>) -> Result<PathBuf, CliError> {
    let path = match path {
        Some(path) => path,
        None => env::current_dir().map_err(CliError::CurrentDir)?,
    };
    let canonical =
        fs::canonicalize(&path).map_err(|_| CliError::WorkspaceNotFound { path: path.clone() })?;

    if !canonical.is_dir() {
        return Err(CliError::WorkspaceNotDirectory { path: canonical });
    }

    if canonical.to_str().is_none() {
        return Err(CliError::WorkspacePathNonUtf8 { path: canonical });
    }

    Ok(canonical)
}

/// macOSアプリを起動して Workspace 切り替え要求を渡す
fn launch_app_for_workspace(workspace_path: &Path) -> Result<(), CliError> {
    let workspace_path = workspace_path
        .to_str()
        .ok_or_else(|| CliError::WorkspacePathNonUtf8 {
            path: workspace_path.to_path_buf(),
        })?;

    launch_app_with_open_command(workspace_path)
}

/// macOS の open コマンドでアプリを起動する
#[cfg(target_os = "macos")]
fn launch_app_with_open_command(workspace_path: &str) -> Result<(), CliError> {
    let output = ProcessCommand::new("/usr/bin/open")
        .args(app_open_command_args(workspace_path))
        .output()
        .map_err(CliError::AppLaunchIo)?;

    if output.status.success() {
        return Ok(());
    }

    Err(app_launch_error_from_output(
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).as_ref(),
    ))
}

/// macOS 以外ではアプリ起動を非対応として扱う
#[cfg(not(target_os = "macos"))]
fn launch_app_with_open_command(_workspace_path: &str) -> Result<(), CliError> {
    Err(CliError::AppOpenUnsupported)
}

/// Fwd Deck.app 起動用の open 引数を生成する
#[cfg(any(test, target_os = "macos"))]
fn app_open_command_args(workspace_path: &str) -> Vec<String> {
    vec![
        "-n".to_owned(),
        "-b".to_owned(),
        FWD_DECK_APP_BUNDLE_IDENTIFIER.to_owned(),
        "--args".to_owned(),
        FWD_DECK_APP_OPEN_WORKSPACE_ARG.to_owned(),
        workspace_path.to_owned(),
    ]
}

/// open コマンドの失敗をユーザー向けエラーへ変換する
#[cfg(any(test, target_os = "macos"))]
fn app_launch_error_from_output(status_code: Option<i32>, stderr: &str) -> CliError {
    if app_launch_error_indicates_missing_app(stderr) {
        return CliError::AppNotInstalled;
    }

    let detail = stderr.trim();
    let message = if detail.is_empty() {
        match status_code {
            Some(code) => format!("/usr/bin/open exited with status {code}."),
            None => "/usr/bin/open was terminated by signal.".to_owned(),
        }
    } else {
        format!("/usr/bin/open の出力:\n{detail}")
    };

    CliError::AppLaunchFailed { message }
}

/// LaunchServices の出力がアプリ未検出を示すか判定する
#[cfg(any(test, target_os = "macos"))]
fn app_launch_error_indicates_missing_app(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();

    stderr.contains("-10814")
        || stderr.contains("lscopyapplicationurlsforbundleidentifier() failed")
        || stderr.contains("unable to find application")
        || stderr.contains("application not found")
        || stderr.contains("was not found")
}

/// 設定編集コマンドを実行する
fn config_command(paths: &ConfigPaths, command: ConfigCommand) -> Result<ExitCode, CliError> {
    match command {
        ConfigCommand::Add { scope } => config_add_command(paths, scope),
        ConfigCommand::Remove { scope } => config_remove_command(paths, scope),
        ConfigCommand::Edit { id, scope } => config_edit_command(paths, &id, scope),
    }
}

/// 設定ファイルへトンネルを追加する
fn config_add_command(
    paths: &ConfigPaths,
    scope: Option<ConfigScopeArg>,
) -> Result<ExitCode, CliError> {
    let scope = resolve_config_scope(paths, scope)?;
    let path = config_path_for_scope(paths, scope)?;
    let config = load_effective_config(paths)?;
    let tunnel = prompt_tunnel_config(&config, scope)?;

    add_tunnel_to_config_file(&path, scope, tunnel)?;
    println!(
        "{}",
        green(
            &format!(
                "Added tunnel to {} configuration: {}",
                scope,
                format_path_for_display(&path)
            ),
            OutputStream::Stdout
        )
    );

    Ok(ExitCode::SUCCESS)
}

/// 設定ファイルからトンネルを削除する
fn config_remove_command(
    paths: &ConfigPaths,
    scope: Option<ConfigScopeArg>,
) -> Result<ExitCode, CliError> {
    let scope = resolve_config_scope(paths, scope)?;
    let path = config_path_for_scope(paths, scope)?;
    let Some(file) = read_config_file(&path, scope)? else {
        eprintln!(
            "{}",
            red(
                &format!(
                    "Configuration file was not found: {}",
                    format_path_for_display(&path)
                ),
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    };

    if file.tunnels.is_empty() {
        println!(
            "No tunnels are configured in {}.",
            format_path_for_display(&path)
        );
        return Ok(ExitCode::SUCCESS);
    }

    let choice = prompt_tunnel_to_remove(&file.tunnels)?;
    let confirmed = Confirm::new(&format!("Remove tunnel '{}'?", choice.id))
        .with_default(false)
        .prompt()?;

    if !confirmed {
        println!("No tunnel was removed.");
        return Ok(ExitCode::SUCCESS);
    }

    remove_tunnel_from_config_file(&path, scope, &choice.id)?;
    println!(
        "{}",
        green(
            &format!("Removed tunnel from {} configuration: {}", scope, choice.id),
            OutputStream::Stdout
        )
    );

    Ok(ExitCode::SUCCESS)
}

/// 設定ファイル内の既存トンネルを更新する
fn config_edit_command(
    paths: &ConfigPaths,
    id: &str,
    scope: Option<ConfigScopeArg>,
) -> Result<ExitCode, CliError> {
    let target = resolve_config_edit_target(paths, id, scope)?;
    let config = load_effective_config(paths)?;
    let tunnel = prompt_tunnel_config_update(&config, &target.tunnel, target.scope)?;

    update_tunnel_in_config_file(&target.path, target.scope, id, tunnel)?;
    println!(
        "{}",
        green(
            &format!(
                "Updated tunnel in {} configuration: {}",
                target.scope,
                format_path_for_display(&target.path)
            ),
            OutputStream::Stdout
        )
    );

    Ok(ExitCode::SUCCESS)
}

/// CLI 指定または対話選択から設定スコープを解決する
fn resolve_config_scope(
    paths: &ConfigPaths,
    scope: Option<ConfigScopeArg>,
) -> Result<ConfigSourceKind, CliError> {
    if let Some(scope) = scope {
        return Ok(scope.into());
    }

    Ok(prompt_config_scope(paths)?)
}

/// 対象スコープの設定ファイルパスを取得する
fn config_path_for_scope(
    paths: &ConfigPaths,
    scope: ConfigSourceKind,
) -> Result<PathBuf, CliError> {
    match scope {
        ConfigSourceKind::Global => paths
            .global
            .clone()
            .ok_or(CliError::MissingGlobalConfigPath),
        ConfigSourceKind::Local => Ok(paths.local.clone()),
    }
}

/// 設定編集対象として解決済みのファイルとトンネルを表現する
#[derive(Debug, Clone)]
struct ConfigEditTarget {
    scope: ConfigSourceKind,
    path: PathBuf,
    tunnel: TunnelConfig,
}

/// 既存トンネルの編集対象ファイルを解決する
fn resolve_config_edit_target(
    paths: &ConfigPaths,
    id: &str,
    scope: Option<ConfigScopeArg>,
) -> Result<ConfigEditTarget, CliError> {
    if let Some(scope) = scope {
        let scope = ConfigSourceKind::from(scope);
        return config_edit_target_for_scope(paths, id, scope);
    }

    let targets = config_edit_targets_for_id(paths, id)?;
    match targets.len() {
        0 => Err(ConfigEditError::NotFound {
            path: paths.local.clone(),
            name: id.to_owned(),
        }
        .into()),
        1 => Ok(targets
            .into_iter()
            .next()
            .expect("single edit target should exist")),
        _ if io::stdin().is_terminal() => prompt_config_edit_target(targets),
        _ => Err(CliError::AmbiguousConfigEdit { id: id.to_owned() }),
    }
}

/// 指定スコープから編集対象トンネルを取得する
fn config_edit_target_for_scope(
    paths: &ConfigPaths,
    id: &str,
    scope: ConfigSourceKind,
) -> Result<ConfigEditTarget, CliError> {
    let path = config_path_for_scope(paths, scope)?;
    let Some(file) = read_config_file(&path, scope)? else {
        return Err(ConfigEditError::Missing { path }.into());
    };
    let Some(tunnel) = file.tunnels.into_iter().find(|tunnel| tunnel.name == id) else {
        return Err(ConfigEditError::NotFound {
            path,
            name: id.to_owned(),
        }
        .into());
    };

    Ok(ConfigEditTarget {
        scope,
        path,
        tunnel,
    })
}

/// 全スコープから指定 ID の編集対象を取得する
fn config_edit_targets_for_id(
    paths: &ConfigPaths,
    id: &str,
) -> Result<Vec<ConfigEditTarget>, CliError> {
    let mut targets = Vec::new();

    if let Some(global_path) = &paths.global
        && let Some(file) = read_config_file(global_path, ConfigSourceKind::Global)?
        && let Some(tunnel) = file.tunnels.into_iter().find(|tunnel| tunnel.name == id)
    {
        targets.push(ConfigEditTarget {
            scope: ConfigSourceKind::Global,
            path: global_path.clone(),
            tunnel,
        });
    }

    if let Some(file) = read_config_file(&paths.local, ConfigSourceKind::Local)?
        && let Some(tunnel) = file.tunnels.into_iter().find(|tunnel| tunnel.name == id)
    {
        targets.push(ConfigEditTarget {
            scope: ConfigSourceKind::Local,
            path: paths.local.clone(),
            tunnel,
        });
    }

    Ok(targets)
}

/// 複数スコープに存在するトンネルの編集対象を対話的に選択する
fn prompt_config_edit_target(targets: Vec<ConfigEditTarget>) -> Result<ConfigEditTarget, CliError> {
    let choices = targets
        .into_iter()
        .map(EditTargetChoice::new)
        .collect::<Vec<_>>();
    let selected = Select::new("Select configuration entry to edit:", choices).prompt()?;

    Ok(selected.target)
}

/// トンネル設定一覧コマンドを実行する
fn list_command(
    config: &EffectiveConfig,
    tags: Vec<String>,
    query: Option<String>,
    wide: bool,
    json: bool,
) -> Result<ExitCode, CliError> {
    let tags = normalize_cli_tags(&tags)?;
    let query = normalize_list_query(query);

    if !config.has_sources() {
        if json {
            print_json(&ListJson::empty())?;
            return Ok(ExitCode::SUCCESS);
        }

        println!(
            "{}",
            red("No configuration files were found.", OutputStream::Stdout)
        );
        return Ok(ExitCode::SUCCESS);
    }

    let tunnels = select_tunnels_for_list(config, &tags, query.as_deref());

    if json {
        print_json(&ListJson::from_tunnels(&tunnels))?;
        return Ok(ExitCode::SUCCESS);
    }

    if tunnels.is_empty() && tags.is_empty() && query.is_none() {
        println!("No tunnels are configured.");
        return Ok(ExitCode::SUCCESS);
    }

    if tunnels.is_empty() {
        println!("{}", no_list_matches_message(&tags, query.as_deref()));
        return Ok(ExitCode::SUCCESS);
    }

    print_list(&tunnels, wide);
    Ok(ExitCode::SUCCESS)
}

/// トンネル詳細表示コマンドを実行する
fn show_command(
    config: &EffectiveConfig,
    name: &str,
    scope: Option<ConfigScopeArg>,
    json: bool,
) -> Result<ExitCode, CliError> {
    if !config.has_sources() {
        eprintln!(
            "{}",
            red("No configuration files were found.", OutputStream::Stderr)
        );
        return Ok(ExitCode::FAILURE);
    }

    let Some(resolved) = find_tunnel_by_name(config, name, scope) else {
        eprintln!(
            "{}",
            red(
                &format!("No tunnel matched NAME: {name}."),
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    };

    if json {
        print_json(&ShowJson::from_tunnel(resolved))?;
        return Ok(ExitCode::SUCCESS);
    }

    print_tunnel_details(resolved);
    Ok(ExitCode::SUCCESS)
}

/// 統合済みトンネル設定の一覧を表示する
fn print_list(tunnels: &[&ResolvedTunnelConfig], wide: bool) {
    let rows = list_rows(tunnels, wide);
    let widths = list_column_widths(&rows);

    println!(
        "{} {} {} {} {} SOURCE",
        pad_display_width("NAME", widths.id),
        pad_display_width("LOCAL", widths.local),
        pad_display_width("REMOTE", widths.remote),
        pad_display_width("SSH", widths.ssh),
        pad_display_width("TAGS", widths.tags)
    );

    for row in &rows {
        print_tunnel_row(row, widths);
    }
}

/// list 表示用の行を生成する
fn list_rows<'a>(tunnels: &[&'a ResolvedTunnelConfig], wide: bool) -> Vec<ListRow<'a>> {
    tunnels
        .iter()
        .map(|resolved| ListRow::from_resolved_tunnel(resolved, wide))
        .collect()
}

/// トンネル設定の一覧行を表示する
fn print_tunnel_row(row: &ListRow<'_>, widths: ListColumnWidths) {
    println!(
        "{} {} {} {} {} {}",
        pad_display_width_with_visible_width(row.id, row.id_width, widths.id),
        pad_display_width_with_visible_width(&row.local, row.local_width, widths.local),
        pad_display_width_with_visible_width(&row.remote, row.remote_width, widths.remote),
        pad_display_width_with_visible_width(&row.ssh, row.ssh_width, widths.ssh),
        pad_display_width_with_visible_width(&row.tags, row.tags_width, widths.tags),
        row.source
    );
}

/// list 表示用に事前整形した 1 行を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListRow<'a> {
    id: &'a str,
    id_width: usize,
    local: String,
    local_width: usize,
    remote: String,
    remote_width: usize,
    ssh: String,
    ssh_width: usize,
    tags: String,
    tags_width: usize,
    source: ConfigSourceKind,
}

impl<'a> ListRow<'a> {
    /// 統合済みトンネル設定から list 表示行を生成する
    fn from_resolved_tunnel(resolved: &'a ResolvedTunnelConfig, wide: bool) -> Self {
        let tunnel = &resolved.tunnel;
        let local = format_local_endpoint(tunnel);
        let remote = format_list_remote_endpoint(tunnel, wide);
        let ssh = format_ssh_endpoint(tunnel);
        let tags = format_tag_list(&tunnel.tags);

        Self {
            id: tunnel.name.as_str(),
            id_width: display_width(&tunnel.name),
            local_width: display_width(&local),
            local,
            remote_width: display_width(&remote),
            remote,
            ssh_width: display_width(&ssh),
            ssh,
            tags_width: display_width(&tags),
            tags,
            source: resolved.source.kind,
        }
    }
}

/// list 表示の列幅を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListColumnWidths {
    id: usize,
    local: usize,
    remote: usize,
    ssh: usize,
    tags: usize,
}

impl ListColumnWidths {
    /// 最小表示幅を初期値として列幅を初期化する
    fn with_minimums() -> Self {
        Self {
            id: LIST_ID_MIN_WIDTH,
            local: LIST_LOCAL_MIN_WIDTH,
            remote: LIST_REMOTE_MIN_WIDTH,
            ssh: LIST_SSH_MIN_WIDTH,
            tags: LIST_TAGS_MIN_WIDTH,
        }
    }
}

/// list 表示対象の値から列幅を算出する
fn list_column_widths(rows: &[ListRow<'_>]) -> ListColumnWidths {
    rows.iter()
        .fold(ListColumnWidths::with_minimums(), |widths, row| {
            ListColumnWidths {
                id: widths.id.max(row.id_width),
                local: widths.local.max(row.local_width),
                remote: widths.remote.max(row.remote_width),
                ssh: widths.ssh.max(row.ssh_width),
                tags: widths.tags.max(row.tags_width),
            }
        })
}

/// list 表示用の remote endpoint 文字列を生成する
fn format_list_remote_endpoint(tunnel: &TunnelConfig, wide: bool) -> String {
    let remote_host = if wide {
        tunnel.remote_host.clone()
    } else {
        truncate_display_width(&tunnel.remote_host, LIST_REMOTE_HOST_MAX_WIDTH)
    };

    format!("{}:{}", remote_host, tunnel.remote_port)
}

/// 指定表示幅を超える文字列を末尾省略する
fn truncate_display_width(value: &str, max_width: usize) -> String {
    if display_width(value) <= max_width {
        return value.to_owned();
    }

    let marker_width = display_width(TRUNCATION_MARKER);
    if max_width <= marker_width {
        return TRUNCATION_MARKER
            .chars()
            .take(max_width)
            .collect::<String>();
    }

    let content_width = max_width - marker_width;
    let mut truncated = String::new();
    let mut current_width = 0;

    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current_width + character_width > content_width {
            break;
        }

        truncated.push(character);
        current_width += character_width;
    }

    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

/// 端末上の表示幅に基づいて文字列を左詰めする
fn pad_display_width(value: &str, width: usize) -> String {
    let display_width = display_width(value);

    pad_display_width_with_visible_width(value, display_width, width)
}

/// 端末上の表示幅と描画文字列を分けて左詰めする
fn pad_display_width_with_visible_width(value: &str, visible_width: usize, width: usize) -> String {
    if visible_width >= width {
        return value.to_owned();
    }

    let padding_width = width - visible_width;
    let mut padded = String::with_capacity(value.len() + padding_width);
    padded.push_str(value);
    padded.push_str(&" ".repeat(padding_width));
    padded
}

/// 端末上で占有する表示幅を取得する
fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

/// トンネル設定の詳細を表示する
fn print_tunnel_details(resolved: &ResolvedTunnelConfig) {
    let tunnel = &resolved.tunnel;

    println!("NAME: {}", tunnel.name);
    println!(
        "Description: {}",
        format_optional_value(tunnel.description.as_deref())
    );
    println!("Tags: {}", format_tag_list(&tunnel.tags));
    println!("Local: {}", format_local_endpoint(tunnel));
    println!("Remote: {}", format_remote_endpoint(tunnel));
    println!("SSH: {}", format_ssh_endpoint(tunnel));
    println!(
        "Identity file: {}",
        format_optional_value(tunnel.identity_file.as_deref())
    );
    println!(
        "Source: {} ({})",
        resolved.source.kind,
        format_path_for_display(&resolved.source.path)
    );
    println!("Runtime ID: {}", runtime_id_for_resolved_tunnel(resolved));
    print_timeout_details(resolved.timeouts);
}

/// トンネル設定の local endpoint 表示文字列を生成する
fn format_local_endpoint(tunnel: &TunnelConfig) -> String {
    format!("{}:{}", tunnel.effective_local_host(), tunnel.local_port)
}

/// トンネル設定の remote endpoint 表示文字列を生成する
fn format_remote_endpoint(tunnel: &TunnelConfig) -> String {
    format!("{}:{}", tunnel.remote_host, tunnel.remote_port)
}

/// トンネル設定の SSH 接続先表示文字列を生成する
fn format_ssh_endpoint(tunnel: &TunnelConfig) -> String {
    match tunnel.ssh_port {
        Some(port) => format!("{}@{}:{}", tunnel.ssh_user, tunnel.ssh_host, port),
        None => format!("{}@{}", tunnel.ssh_user, tunnel.ssh_host),
    }
}

/// タグ一覧を表示用文字列へ変換する
fn format_tag_list(tags: &[String]) -> String {
    if tags.is_empty() {
        "-".to_owned()
    } else {
        tags.join(",")
    }
}

/// 任意入力値を表示用文字列へ変換する
fn format_optional_value(value: Option<&str>) -> &str {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("-")
}

/// 解決済みタイムアウト設定を表示する
fn print_timeout_details(timeouts: ResolvedTimeoutConfig) {
    println!("Timeouts:");
    println!("  Connect timeout: {}s", timeouts.connect_timeout_seconds);
    println!(
        "  Server alive interval: {}s",
        timeouts.server_alive_interval_seconds
    );
    println!(
        "  Server alive count max: {}",
        timeouts.server_alive_count_max
    );
    println!("  Start grace: {}ms", timeouts.start_grace_milliseconds);
}

/// トンネル開始コマンドを実行する
fn start_command(
    config: &EffectiveConfig,
    state_path: &Path,
    options: StartCommandOptions,
) -> Result<ExitCode, CliError> {
    let StartCommandOptions {
        names,
        tags,
        all,
        parallel,
        dry_run,
        scope,
        json,
    } = options;

    if !config.has_sources() {
        eprintln!(
            "{}",
            red("No configuration files were found.", OutputStream::Stderr)
        );
        return Ok(ExitCode::FAILURE);
    }

    if !print_validation_if_invalid(config) {
        return Ok(ExitCode::FAILURE);
    }

    if all && (!names.is_empty() || !tags.is_empty()) {
        eprintln!(
            "{}",
            red(
                "Cannot combine --all with tunnel NAMEs or --tag.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    if !names.is_empty() && !tags.is_empty() {
        eprintln!(
            "{}",
            red(
                "Cannot combine tunnel NAMEs with --tag.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    if parallel == 0 {
        eprintln!(
            "{}",
            red(
                "Parallelism must be greater than or equal to 1.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    let tags = normalize_cli_tags(&tags)?;

    let tunnels = if all {
        select_tunnels_for_scope(sorted_tunnels_by_id(&config.tunnels), scope)
    } else if !tags.is_empty() {
        let tunnels = select_tunnels_for_scope(select_tunnels_for_tags(config, &tags), scope);

        if tunnels.is_empty() {
            eprintln!(
                "{}",
                red(
                    &format!("No tunnels matched tags: {}.", tags.join(", ")),
                    OutputStream::Stderr
                )
            );
            return Ok(ExitCode::FAILURE);
        }

        tunnels
    } else if names.is_empty() {
        let runtime_ids = prompt_tunnels_to_start(config, scope)?;
        find_tunnels_by_runtime_ids(config, &runtime_ids)
    } else {
        let Ok(tunnels) = find_tunnels_by_names(config, &names, scope) else {
            print_unknown_names(config, &names, scope);
            return Ok(ExitCode::FAILURE);
        };
        tunnels
    };

    if tunnels.is_empty() {
        println!("No tunnels were selected.");
        return Ok(ExitCode::SUCCESS);
    }

    if dry_run {
        if json {
            print_json(&StartDryRunJson::from_tunnels(&tunnels, state_path))?;
        } else {
            print_start_dry_run(&tunnels, state_path);
        }
        return Ok(ExitCode::SUCCESS);
    }

    if json {
        return Err(CliError::JsonUnsupported);
    }

    let mut failed = false;
    let resolved_tunnels = tunnels.into_iter().cloned().collect::<Vec<_>>();

    for result in start_tunnels(&resolved_tunnels, state_path, parallel)? {
        match result {
            Ok(started) => print_started_tunnel(&started),
            Err(error) => {
                failed = true;
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }

    Ok(exit_code_from_failure(failed))
}

/// start コマンドの実行条件を保持する
#[derive(Debug, Clone)]
struct StartCommandOptions {
    names: Vec<String>,
    tags: Vec<String>,
    all: bool,
    parallel: usize,
    dry_run: bool,
    scope: Option<ConfigScopeArg>,
    json: bool,
}

/// stale なトンネルを再起動する
fn recover_command(
    config: &EffectiveConfig,
    state_path: &Path,
    names: Vec<String>,
    scope: Option<ConfigScopeArg>,
) -> Result<ExitCode, CliError> {
    if !config.has_sources() {
        eprintln!(
            "{}",
            red("No configuration files were found.", OutputStream::Stderr)
        );
        return Ok(ExitCode::FAILURE);
    }

    if !print_validation_if_invalid(config) {
        return Ok(ExitCode::FAILURE);
    }

    let statuses = tunnel_statuses(state_path)?;

    if statuses.is_empty() {
        println!("No tracked tunnels.");
        return Ok(ExitCode::SUCCESS);
    }

    let recovery_runtime_ids = if names.is_empty() {
        stale_runtime_ids(&statuses, scope)
    } else {
        let Ok(tunnels) = find_tunnels_by_names(config, &names, scope) else {
            print_unknown_names(config, &names, scope);
            return Ok(ExitCode::FAILURE);
        };
        tunnels
            .into_iter()
            .map(runtime_id_for_resolved_tunnel)
            .collect()
    };

    if recovery_runtime_ids.is_empty() {
        println!("No stale tunnels to recover.");
        return Ok(ExitCode::SUCCESS);
    }

    let statuses_by_runtime_id = status_index_by_runtime_id(&statuses);
    let mut failed = false;

    for runtime_id in recovery_runtime_ids {
        let Some(status) = statuses_by_runtime_id.get(runtime_id.as_str()).copied() else {
            failed = true;
            eprintln!(
                "{}",
                red(
                    &format!("Tunnel is not tracked: {runtime_id}"),
                    OutputStream::Stderr
                )
            );
            continue;
        };

        if status.process_state == ProcessState::Running {
            println!(
                "{}",
                green(
                    &format!(
                        "Tunnel is already running: {} (pid: {})",
                        status.state.name, status.state.pid
                    ),
                    OutputStream::Stdout
                )
            );
            continue;
        }

        let Some(tunnel) = find_tunnel_by_runtime_id(config, &runtime_id) else {
            failed = true;
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Configured tunnel not found for stale state: {} ({runtime_id})",
                        status.state.name
                    ),
                    OutputStream::Stderr
                )
            );
            continue;
        };

        match start_tunnel(tunnel, state_path) {
            Ok(started) => print_started_tunnel(&started),
            Err(error) => {
                failed = true;
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }

    Ok(exit_code_from_failure(failed))
}

/// 追跡中トンネルを監視して stale なトンネルを再起動する
fn watch_command(
    config: &EffectiveConfig,
    state_path: &Path,
    names: Vec<String>,
    scope: Option<ConfigScopeArg>,
    interval_seconds: u64,
) -> Result<ExitCode, CliError> {
    if !config.has_sources() {
        eprintln!(
            "{}",
            red("No configuration files were found.", OutputStream::Stderr)
        );
        return Ok(ExitCode::FAILURE);
    }

    if !print_validation_if_invalid(config) {
        return Ok(ExitCode::FAILURE);
    }

    if interval_seconds == 0 {
        eprintln!(
            "{}",
            red(
                "Watch interval must be greater than or equal to 1 second.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    let runtime_ids = if names.is_empty() {
        Vec::new()
    } else {
        let Ok(tunnels) = find_tunnels_by_names(config, &names, scope) else {
            print_unknown_names(config, &names, scope);
            return Ok(ExitCode::FAILURE);
        };
        tunnels
            .into_iter()
            .map(runtime_id_for_resolved_tunnel)
            .collect()
    };

    print_watch_started(&names, interval_seconds);
    let interval = Duration::from_secs(interval_seconds);

    loop {
        let statuses = tunnel_statuses(state_path)?;
        let stale_runtime_ids = watched_stale_runtime_ids(&statuses, &runtime_ids, scope);

        if !stale_runtime_ids.is_empty() {
            recover_watch_stale_tunnels(config, state_path, &stale_runtime_ids)?;
        }

        thread::sleep(interval);
    }
}

/// トンネル状態表示コマンドを実行する
fn status_command(state_path: &Path, json: bool) -> Result<ExitCode, CliError> {
    let statuses = tunnel_statuses(state_path)?;

    if json {
        let now = current_unix_seconds();
        let statuses = sorted_statuses_by_id(&statuses);
        print_json(&StatusJson::from_statuses(state_path, &statuses, now))?;
        return Ok(ExitCode::SUCCESS);
    }

    if statuses.is_empty() {
        println!("No tracked tunnels.");
        return Ok(ExitCode::SUCCESS);
    }

    let now = current_unix_seconds();
    let statuses = sorted_statuses_by_id(&statuses);
    let rows = status_rows(&statuses, now);
    let widths = status_column_widths(&rows);

    print_status_header(widths);

    for row in &rows {
        print_status_row(row, widths);
    }

    Ok(ExitCode::SUCCESS)
}

/// トンネル停止コマンドを実行する
fn stop_command(
    state_path: &Path,
    names: Vec<String>,
    all: bool,
    dry_run: bool,
    scope: Option<ConfigScopeArg>,
) -> Result<ExitCode, CliError> {
    let statuses = tunnel_statuses(state_path)?;

    if all && !names.is_empty() {
        eprintln!(
            "{}",
            red(
                "Cannot combine --all with tunnel NAMEs.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    if statuses.is_empty() && names.is_empty() {
        println!("No tracked tunnels.");
        return Ok(ExitCode::SUCCESS);
    }

    let runtime_ids = if all {
        sorted_statuses_by_id(&statuses)
            .into_iter()
            .filter(|status| status_matches_scope(status, scope))
            .map(|status| status.state.runtime_id.clone())
            .collect()
    } else if names.is_empty() {
        prompt_tunnels_to_stop(&statuses, scope)?
    } else {
        let Ok(statuses) = find_statuses_by_names(&statuses, &names, scope) else {
            print_untracked_names(&statuses, &names, scope);
            return Ok(ExitCode::FAILURE);
        };
        statuses
            .into_iter()
            .map(|status| status.state.runtime_id.clone())
            .collect()
    };

    if runtime_ids.is_empty() {
        println!("No tunnels were selected.");
        return Ok(ExitCode::SUCCESS);
    }

    if dry_run {
        return Ok(print_stop_dry_run(&statuses, &runtime_ids, state_path));
    }

    let mut failed = false;

    for runtime_id in runtime_ids {
        match stop_tunnel(&runtime_id, state_path) {
            Ok(stopped) => print_stopped_tunnel(&stopped),
            Err(error) => {
                failed = true;
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }

    Ok(exit_code_from_failure(failed))
}

/// ローカル実行環境と設定内容を診断する
fn doctor_command(
    config: &EffectiveConfig,
    paths: &ConfigPaths,
    state_path: &Path,
) -> Result<ExitCode, CliError> {
    let checks = doctor_checks(config, paths, state_path);
    let failed = checks
        .iter()
        .any(|check| check.status == DoctorCheckStatus::Error);

    println!("Doctor report");
    for check in &checks {
        println!(
            "{} {}: {}",
            doctor_status_label(check.status),
            check.name,
            check.message
        );
    }

    Ok(exit_code_from_failure(failed))
}

/// doctor で実行する診断項目を生成する
fn doctor_checks(
    config: &EffectiveConfig,
    paths: &ConfigPaths,
    state_path: &Path,
) -> Vec<DoctorCheck> {
    let mut checks = vec![
        doctor_config_presence_check(config, paths),
        doctor_validation_check(config),
        doctor_state_check(state_path),
        doctor_command_check("ssh", &["-V"]),
        doctor_command_check("lsof", &["-v"]),
    ];
    checks.extend(doctor_identity_file_checks(config));
    checks.extend(doctor_local_endpoint_checks(config));

    checks
}

/// 設定ファイルの有無を診断する
fn doctor_config_presence_check(config: &EffectiveConfig, paths: &ConfigPaths) -> DoctorCheck {
    if config.has_sources() {
        return DoctorCheck::ok(
            "Configuration files",
            format!("loaded {} file(s)", config.sources.len()),
        );
    }

    DoctorCheck::error(
        "Configuration files",
        format!(
            "no configuration files were found (global: {}, local: {})",
            paths
                .global
                .as_deref()
                .map(format_path_for_display)
                .unwrap_or_else(|| "-".to_owned()),
            format_path_for_display(&paths.local)
        ),
    )
}

/// 設定検証結果を診断する
fn doctor_validation_check(config: &EffectiveConfig) -> DoctorCheck {
    let report = validate_config(config);

    if report.is_valid() {
        if report.has_warnings() {
            return DoctorCheck::warning(
                "Configuration validation",
                format!("valid with {} warning(s)", report.warnings.len()),
            );
        }

        return DoctorCheck::ok("Configuration validation", "valid");
    }

    DoctorCheck::error(
        "Configuration validation",
        format!("{} error(s)", report.errors.len()),
    )
}

/// 状態ファイルの読み書きを診断する
fn doctor_state_check(state_path: &Path) -> DoctorCheck {
    if let Err(error) = fwd_deck_core::state::read_state_file(state_path) {
        return DoctorCheck::error("State file", error.to_string());
    }

    match verify_state_file_writable(state_path) {
        Ok(()) => DoctorCheck::ok(
            "State file",
            format!(
                "readable and writable: {}",
                format_path_for_display(state_path)
            ),
        ),
        Err(error) => DoctorCheck::error("State file", error),
    }
}

/// 状態ファイルと同じ場所へ一時ファイルを書き込めるか検証する
fn verify_state_file_writable(state_path: &Path) -> Result<(), String> {
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            format!(
                "failed to create state directory {}: {source}",
                format_path_for_display(parent)
            )
        })?;
    }

    let temp_path = doctor_state_temp_path(state_path);
    fs::write(&temp_path, "tunnels = []\n").map_err(|source| {
        format!(
            "failed to write {}: {source}",
            format_path_for_display(&temp_path)
        )
    })?;
    fs::read_to_string(&temp_path).map_err(|source| {
        format!(
            "failed to read {}: {source}",
            format_path_for_display(&temp_path)
        )
    })?;
    fs::remove_file(&temp_path).map_err(|source| {
        format!(
            "failed to remove {}: {source}",
            format_path_for_display(&temp_path)
        )
    })?;

    Ok(())
}

/// doctor 用の一時状態ファイルパスを生成する
fn doctor_state_temp_path(state_path: &Path) -> PathBuf {
    let file_name = state_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state.toml");

    state_path.with_file_name(format!("{file_name}.doctor.{}.tmp", std::process::id()))
}

/// 外部コマンドの起動可否を診断する
fn doctor_command_check(command: &'static str, args: &[&str]) -> DoctorCheck {
    match ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => DoctorCheck::ok(format!("{command} command"), "available"),
        Err(error) if error.kind() == io::ErrorKind::NotFound && command == "lsof" => {
            DoctorCheck::warning(
                "lsof command",
                "not found; port process details may be limited",
            )
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            DoctorCheck::error(format!("{command} command"), "not found")
        }
        Err(error) => DoctorCheck::error(format!("{command} command"), error.to_string()),
    }
}

/// identity_file の存在を診断する
fn doctor_identity_file_checks(config: &EffectiveConfig) -> Vec<DoctorCheck> {
    config
        .tunnels
        .iter()
        .filter_map(|resolved| {
            let identity_file = resolved.tunnel.identity_file.as_deref()?;
            let path = expand_home_pathbuf(identity_file);
            let name = format!("Identity file ({})", resolved.tunnel.name);

            if path.is_file() {
                Some(DoctorCheck::ok(name, format_path_for_display(&path)))
            } else {
                Some(DoctorCheck::error(
                    name,
                    format!("not found: {}", format_path_for_display(&path)),
                ))
            }
        })
        .collect()
}

/// ローカルエンドポイントの使用状況を診断する
fn doctor_local_endpoint_checks(config: &EffectiveConfig) -> Vec<DoctorCheck> {
    config
        .tunnels
        .iter()
        .map(|resolved| {
            let tunnel = &resolved.tunnel;
            let local_host = tunnel.effective_local_host();
            let name = format!("Local endpoint ({})", tunnel.name);

            match TcpListener::bind((local_host, tunnel.local_port)) {
                Ok(listener) => {
                    drop(listener);
                    DoctorCheck::ok(
                        name,
                        format!("{local_host}:{} is available", tunnel.local_port),
                    )
                }
                Err(error) => DoctorCheck::warning(
                    name,
                    format!(
                        "{local_host}:{} is not available: {error}",
                        tunnel.local_port
                    ),
                ),
            }
        })
        .collect()
}

/// `~/` で始まるパスを PathBuf へ展開する
fn expand_home_pathbuf(path: &str) -> PathBuf {
    let Some(rest) = path.strip_prefix("~/") else {
        return PathBuf::from(path);
    };

    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(rest))
        .unwrap_or_else(|| PathBuf::from(path))
}

/// doctor の診断結果を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorCheck {
    status: DoctorCheckStatus,
    name: String,
    message: String,
}

impl DoctorCheck {
    /// 正常な診断結果を生成する
    fn ok(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: DoctorCheckStatus::Ok,
            name: name.into(),
            message: message.into(),
        }
    }

    /// 注意が必要な診断結果を生成する
    fn warning(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: DoctorCheckStatus::Warning,
            name: name.into(),
            message: message.into(),
        }
    }

    /// 失敗した診断結果を生成する
    fn error(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: DoctorCheckStatus::Error,
            name: name.into(),
            message: message.into(),
        }
    }
}

/// doctor の診断結果種別を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoctorCheckStatus {
    Ok,
    Warning,
    Error,
}

/// doctor の診断結果種別を表示ラベルへ変換する
fn doctor_status_label(status: DoctorCheckStatus) -> &'static str {
    match status {
        DoctorCheckStatus::Ok => "[OK]",
        DoctorCheckStatus::Warning => "[WARN]",
        DoctorCheckStatus::Error => "[ERROR]",
    }
}

/// 設定が不正な場合に検証エラーを表示する
fn print_validation_if_invalid(config: &EffectiveConfig) -> bool {
    let report = validate_config(config);

    if report.is_valid() {
        return true;
    }

    print_validation_errors(&report);
    false
}

/// CLI 入力タグを正規化する
fn normalize_cli_tags(tags: &[String]) -> Result<Vec<String>, CliError> {
    tags.iter()
        .map(|tag| normalize_tag_input(tag))
        .collect::<Result<Vec<_>, _>>()
}

/// 1 件のタグ入力を正規化して検証する
fn normalize_tag_input(tag: &str) -> Result<String, CliError> {
    let normalized = normalize_tag(tag);

    if tag_is_valid(&normalized) {
        Ok(normalized)
    } else {
        Err(CliError::InvalidTag {
            tag: display_invalid_tag(tag),
        })
    }
}

/// 空タグを表示用の値へ変換する
fn display_invalid_tag(tag: &str) -> String {
    if tag.trim().is_empty() {
        "<empty>".to_owned()
    } else {
        tag.to_owned()
    }
}

/// タグ指定に応じて統合済みトンネル設定を選択する
fn select_tunnels_for_tags<'a>(
    config: &'a EffectiveConfig,
    tags: &[String],
) -> Vec<&'a ResolvedTunnelConfig> {
    if tags.is_empty() {
        return sorted_tunnels_by_id(&config.tunnels);
    }

    sorted_tunnels_by_id(filter_tunnels_by_tags(&config.tunnels, tags))
}

/// 指定スコープがある場合に統合済みトンネルを絞り込む
fn select_tunnels_for_scope(
    tunnels: Vec<&ResolvedTunnelConfig>,
    scope: Option<ConfigScopeArg>,
) -> Vec<&ResolvedTunnelConfig> {
    let Some(scope) = scope_kind(scope) else {
        return tunnels;
    };

    tunnels
        .into_iter()
        .filter(|resolved| resolved.source.kind == scope)
        .collect()
}

/// list の絞り込み条件に応じて統合済みトンネル設定を選択する
fn select_tunnels_for_list<'a>(
    config: &'a EffectiveConfig,
    tags: &[String],
    query: Option<&str>,
) -> Vec<&'a ResolvedTunnelConfig> {
    select_tunnels_for_tags(config, tags)
        .into_iter()
        .filter(|resolved| {
            query.is_none_or(|query| tunnel_matches_list_query(&resolved.tunnel, query))
        })
        .collect()
}

/// ID 昇順のトンネル一覧を生成する
fn sorted_tunnels_by_id<'a>(
    tunnels: impl IntoIterator<Item = &'a ResolvedTunnelConfig>,
) -> Vec<&'a ResolvedTunnelConfig> {
    let mut tunnels = tunnels.into_iter().collect::<Vec<_>>();
    tunnels.sort_by(|left, right| left.tunnel.name.cmp(&right.tunnel.name));

    tunnels
}

/// list query を比較用の表記へ正規化する
fn normalize_list_query(query: Option<String>) -> Option<String> {
    query
        .map(|query| query.trim().to_ascii_lowercase())
        .filter(|query| !query.is_empty())
}

/// トンネルが list query に一致するかを判定する
fn tunnel_matches_list_query(tunnel: &TunnelConfig, query: &str) -> bool {
    ascii_case_insensitive_contains(&tunnel.name, query)
        || tunnel
            .description
            .as_deref()
            .is_some_and(|description| ascii_case_insensitive_contains(description, query))
}

/// ASCII の大文字小文字を無視して部分一致を判定する
fn ascii_case_insensitive_contains(value: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    value
        .as_bytes()
        .windows(query.len())
        .any(|candidate| candidate.eq_ignore_ascii_case(query.as_bytes()))
}

/// list 絞り込み結果が空の場合のメッセージを生成する
fn no_list_matches_message(tags: &[String], query: Option<&str>) -> String {
    match (tags.is_empty(), query) {
        (false, Some(query)) => {
            format!(
                "No tunnels matched tags and query: {} / {query}.",
                tags.join(", ")
            )
        }
        (false, None) => format!("No tunnels matched tags: {}.", tags.join(", ")),
        (true, Some(query)) => format!("No tunnels matched query: {query}."),
        (true, None) => "No tunnels are configured.".to_owned(),
    }
}

/// 編集対象の設定スコープを対話的に選択する
fn prompt_config_scope(paths: &ConfigPaths) -> Result<ConfigSourceKind, InquireError> {
    let mut choices = Vec::new();

    choices.push(ScopeChoice::new(
        ConfigSourceKind::Local,
        format!("local ({})", format_path_for_display(&paths.local)),
    ));

    if let Some(global_path) = &paths.global {
        choices.push(ScopeChoice::new(
            ConfigSourceKind::Global,
            format!("global ({})", format_path_for_display(global_path)),
        ));
    }

    let selected = Select::new("Select configuration scope:", choices).prompt()?;
    Ok(selected.kind)
}

/// 追加するトンネル設定を対話的に入力する
fn prompt_tunnel_config(
    config: &EffectiveConfig,
    scope: ConfigSourceKind,
) -> Result<TunnelConfig, CliError> {
    let id = prompt_available_tunnel_id(config, scope)?;
    let description = prompt_optional_text("Description:")?;
    let tags = prompt_tags()?;
    let local_host = Some(prompt_local_host()?);
    let local_port = prompt_available_local_port(config, scope, &id)?;
    let remote_host = prompt_required_text("Remote host:")?;
    let remote_port = prompt_port("Remote port:", None)?;
    let ssh_user = prompt_required_text("SSH user:")?;
    let ssh_host = prompt_required_text("SSH host:")?;
    let ssh_port = Some(prompt_port("SSH port:", Some(22))?);
    let identity_file = prompt_optional_text("Identity file:")?;

    Ok(TunnelConfig {
        name: id,
        description,
        tags,
        local_host,
        local_port,
        remote_host,
        remote_port,
        ssh_user,
        ssh_host,
        ssh_port,
        identity_file,
        timeouts: TimeoutConfig::default(),
    })
}

/// 既存値を初期値としてトンネル設定を対話的に更新する
fn prompt_tunnel_config_update(
    config: &EffectiveConfig,
    current: &TunnelConfig,
    scope: ConfigSourceKind,
) -> Result<TunnelConfig, CliError> {
    let id = prompt_available_tunnel_id_for_update(config, scope, &current.name)?;
    let description =
        prompt_existing_optional_text("Description:", current.description.as_deref())?;
    let tags = prompt_existing_tags(&current.tags)?;
    let local_host = prompt_existing_local_host(current)?;
    let local_port =
        prompt_available_local_port_for_update(config, scope, &current.name, current.local_port)?;
    let remote_host = prompt_existing_required_text("Remote host:", &current.remote_host)?;
    let remote_port = prompt_existing_port("Remote port:", current.remote_port)?;
    let ssh_user = prompt_existing_required_text("SSH user:", &current.ssh_user)?;
    let ssh_host = prompt_existing_required_text("SSH host:", &current.ssh_host)?;
    let ssh_port = prompt_existing_optional_port("SSH port:", current.ssh_port)?;
    let identity_file =
        prompt_existing_optional_text("Identity file:", current.identity_file.as_deref())?;

    Ok(TunnelConfig {
        name: id,
        description,
        tags,
        local_host,
        local_port,
        remote_host,
        remote_port,
        ssh_user,
        ssh_host,
        ssh_port,
        identity_file,
        timeouts: current.timeouts.clone(),
    })
}

/// 既存設定と重複しないトンネル ID の入力を受け取る
fn prompt_available_tunnel_id(
    config: &EffectiveConfig,
    scope: ConfigSourceKind,
) -> Result<String, CliError> {
    loop {
        let id = prompt_required_text("Tunnel name:")?;

        if let Some(conflict) = find_tunnel_id_conflict(config, scope, &id) {
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Tunnel name is already used: {} ({}: {})",
                        conflict.tunnel.name,
                        conflict.source.kind,
                        format_path_for_display(&conflict.source.path)
                    ),
                    OutputStream::Stderr
                )
            );
            continue;
        }

        return Ok(id);
    }
}

/// 更新対象を除いて重複しないトンネル ID の入力を受け取る
fn prompt_available_tunnel_id_for_update(
    config: &EffectiveConfig,
    scope: ConfigSourceKind,
    current_id: &str,
) -> Result<String, CliError> {
    loop {
        let id = prompt_existing_required_text("Tunnel name:", current_id)?;

        if let Some(conflict) = find_tunnel_id_conflict_for_update(config, scope, current_id, &id) {
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Tunnel name is already used: {} ({}: {})",
                        conflict.tunnel.name,
                        conflict.source.kind,
                        format_path_for_display(&conflict.source.path)
                    ),
                    OutputStream::Stderr
                )
            );
            continue;
        }

        return Ok(id);
    }
}

/// トンネル ID が重複する既存トンネルを取得する
fn find_tunnel_id_conflict<'a>(
    config: &'a EffectiveConfig,
    scope: ConfigSourceKind,
    tunnel_id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .find(|resolved| resolved.source.kind == scope && resolved.tunnel.name == tunnel_id)
}

/// 更新対象を除いてトンネル ID が重複する既存トンネルを取得する
fn find_tunnel_id_conflict_for_update<'a>(
    config: &'a EffectiveConfig,
    scope: ConfigSourceKind,
    current_id: &str,
    tunnel_id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config.tunnels.iter().find(|resolved| {
        resolved.source.kind == scope
            && resolved.tunnel.name != current_id
            && resolved.tunnel.name == tunnel_id
    })
}

/// 既存設定と重複しない local_port の入力を受け取る
fn prompt_available_local_port(
    config: &EffectiveConfig,
    scope: ConfigSourceKind,
    tunnel_id: &str,
) -> Result<u16, CliError> {
    loop {
        let local_port = prompt_port("Local port:", None)?;

        if let Some(conflict) = find_local_port_conflict(config, scope, tunnel_id, local_port) {
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Local port is already used by tunnel: {} ({}:{})",
                        conflict.tunnel.name,
                        conflict.tunnel.effective_local_host(),
                        conflict.tunnel.local_port
                    ),
                    OutputStream::Stderr
                )
            );
            continue;
        }

        return Ok(local_port);
    }
}

/// 更新対象を除いて重複しない local_port の入力を受け取る
fn prompt_available_local_port_for_update(
    config: &EffectiveConfig,
    scope: ConfigSourceKind,
    current_id: &str,
    current_port: u16,
) -> Result<u16, CliError> {
    loop {
        let local_port = prompt_existing_port("Local port:", current_port)?;

        if let Some(conflict) = find_local_port_conflict(config, scope, current_id, local_port) {
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Local port is already used by tunnel: {} ({}:{})",
                        conflict.tunnel.name,
                        conflict.tunnel.effective_local_host(),
                        conflict.tunnel.local_port
                    ),
                    OutputStream::Stderr
                )
            );
            continue;
        }

        return Ok(local_port);
    }
}

/// local_port が重複する既存トンネルを取得する
fn find_local_port_conflict<'a>(
    config: &'a EffectiveConfig,
    scope: ConfigSourceKind,
    tunnel_id: &str,
    local_port: u16,
) -> Option<&'a ResolvedTunnelConfig> {
    config.tunnels.iter().find(|resolved| {
        resolved.source.kind == scope
            && resolved.tunnel.name != tunnel_id
            && resolved.tunnel.local_port == local_port
    })
}

/// 削除対象のトンネルを対話的に選択する
fn prompt_tunnel_to_remove(tunnels: &[TunnelConfig]) -> Result<RemoveChoice, InquireError> {
    let choices = tunnels
        .iter()
        .map(RemoveChoice::from_tunnel)
        .collect::<Vec<_>>();

    Select::new("Select a tunnel to remove:", choices).prompt()
}

/// 既存値を初期値として空文字列を許容しない入力を受け取る
fn prompt_existing_required_text(label: &str, current: &str) -> Result<String, CliError> {
    loop {
        let value = Text::new(label).with_initial_value(current).prompt()?;
        let trimmed = value.trim();

        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }

        if !current.trim().is_empty() {
            return Ok(current.trim().to_owned());
        }

        eprintln!("{}", red("Value cannot be empty.", OutputStream::Stderr));
    }
}

/// 空文字列を許容しない入力を受け取る
fn prompt_required_text(label: &str) -> Result<String, CliError> {
    loop {
        let value = Text::new(label).prompt()?;
        let trimmed = value.trim();

        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }

        eprintln!("{}", red("Value cannot be empty.", OutputStream::Stderr));
    }
}

/// 空文字列を未指定として扱う入力を受け取る
fn prompt_optional_text(label: &str) -> Result<Option<String>, CliError> {
    let value = Text::new(label).prompt()?;
    let trimmed = value.trim();

    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

/// 既存値を初期値として任意入力値を受け取る
fn prompt_existing_optional_text(
    label: &str,
    current: Option<&str>,
) -> Result<Option<String>, CliError> {
    let mut prompt = Text::new(label);
    if let Some(current) = current {
        prompt = prompt.with_initial_value(current);
    }

    let value = prompt.prompt()?;
    let trimmed = value.trim();

    if trimmed.is_empty() {
        Ok(current.map(ToOwned::to_owned))
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

/// 既存値を初期値としてタグ一覧の入力を受け取る
fn prompt_existing_tags(current: &[String]) -> Result<Vec<String>, CliError> {
    loop {
        let value = Text::new("Tags (comma-separated):")
            .with_initial_value(&current.join(","))
            .prompt()?;

        if value.trim().is_empty() {
            return Ok(current.to_vec());
        }

        match parse_tag_list(&value) {
            Ok(tags) => return Ok(tags),
            Err(error) => {
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }
}

/// タグ一覧の入力を受け取る
fn prompt_tags() -> Result<Vec<String>, CliError> {
    loop {
        let value = Text::new("Tags (comma-separated):").prompt()?;

        match parse_tag_list(&value) {
            Ok(tags) => return Ok(tags),
            Err(error) => {
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }
}

/// カンマ区切りのタグ入力を正規化する
fn parse_tag_list(value: &str) -> Result<Vec<String>, CliError> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }

    value
        .split(',')
        .map(normalize_tag_input)
        .collect::<Result<Vec<_>, _>>()
}

/// 既存値を初期値として local_host の入力を受け取る
fn prompt_existing_local_host(current: &TunnelConfig) -> Result<Option<String>, CliError> {
    loop {
        let local_host =
            prompt_existing_required_text("Local host:", current.effective_local_host())?;

        if !local_host.chars().any(char::is_whitespace) {
            if current.local_host.is_none() && local_host == DEFAULT_LOCAL_HOST {
                return Ok(None);
            }

            return Ok(Some(local_host));
        }

        eprintln!(
            "{}",
            red(
                "Local host cannot contain whitespace.",
                OutputStream::Stderr
            )
        );
    }
}

/// local_host の入力を受け取る
fn prompt_local_host() -> Result<String, CliError> {
    loop {
        let value = prompt_text_with_default("Local host:", DEFAULT_LOCAL_HOST)?;

        if !value.chars().any(char::is_whitespace) {
            return Ok(value);
        }

        eprintln!(
            "{}",
            red(
                "Local host cannot contain whitespace.",
                OutputStream::Stderr
            )
        );
    }
}

/// 既定値つきの入力を受け取る
fn prompt_text_with_default(label: &str, default: &str) -> Result<String, CliError> {
    let value = Text::new(label).with_initial_value(default).prompt()?;
    let trimmed = value.trim();

    if trimmed.is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

/// 既存値を初期値としてポート番号の入力を受け取る
fn prompt_existing_port(label: &str, current: u16) -> Result<u16, CliError> {
    loop {
        let initial_value = current.to_string();
        let value = Text::new(label)
            .with_initial_value(&initial_value)
            .prompt()?;
        let candidate = if value.trim().is_empty() {
            initial_value
        } else {
            value.trim().to_owned()
        };

        if let Ok(port) = candidate.parse::<u16>()
            && port > 0
        {
            return Ok(port);
        }

        eprintln!(
            "{}",
            red(
                "Port must be a number between 1 and 65535.",
                OutputStream::Stderr
            )
        );
    }
}

/// 既存値を初期値として任意のポート番号入力を受け取る
fn prompt_existing_optional_port(
    label: &str,
    current: Option<u16>,
) -> Result<Option<u16>, CliError> {
    loop {
        let mut prompt = Text::new(label);
        let initial_value;
        if let Some(current) = current {
            initial_value = current.to_string();
            prompt = prompt.with_initial_value(&initial_value);
        }

        let value = prompt.prompt()?;
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Ok(current);
        }

        if let Ok(port) = trimmed.parse::<u16>()
            && port > 0
        {
            return Ok(Some(port));
        }

        eprintln!(
            "{}",
            red(
                "Port must be a number between 1 and 65535.",
                OutputStream::Stderr
            )
        );
    }
}

/// ポート番号の入力を受け取る
fn prompt_port(label: &str, default: Option<u16>) -> Result<u16, CliError> {
    loop {
        let value = prompt_port_text(label, default)?;
        let trimmed = value.trim();
        let candidate = if trimmed.is_empty() {
            default.map(|port| port.to_string()).unwrap_or_default()
        } else {
            trimmed.to_owned()
        };

        if let Ok(port) = candidate.parse::<u16>()
            && port > 0
        {
            return Ok(port);
        }

        eprintln!(
            "{}",
            red(
                "Port must be a number between 1 and 65535.",
                OutputStream::Stderr
            )
        );
    }
}

/// ポート番号の文字列入力を受け取る
fn prompt_port_text(label: &str, default: Option<u16>) -> Result<String, CliError> {
    match default {
        Some(default) => {
            let initial_value = default.to_string();
            Ok(Text::new(label)
                .with_initial_value(&initial_value)
                .prompt()?)
        }
        None => Ok(Text::new(label).prompt()?),
    }
}

/// 開始対象のトンネルを対話的に選択する
fn prompt_tunnels_to_start(
    config: &EffectiveConfig,
    scope: Option<ConfigScopeArg>,
) -> Result<Vec<String>, InquireError> {
    let choices = select_tunnels_for_scope(sorted_tunnels_by_id(&config.tunnels), scope)
        .into_iter()
        .map(StartChoice::from_resolved_tunnel)
        .collect::<Vec<_>>();

    if choices.is_empty() {
        return Ok(Vec::new());
    }

    let selected = MultiSelect::new("Select tunnels to start:", choices).prompt()?;

    Ok(selected.into_iter().map(|choice| choice.id).collect())
}

/// 停止対象のトンネルを対話的に選択する
fn prompt_tunnels_to_stop(
    statuses: &[TunnelRuntimeStatus],
    scope: Option<ConfigScopeArg>,
) -> Result<Vec<String>, InquireError> {
    let sorted_statuses = sorted_statuses_by_id(statuses)
        .into_iter()
        .filter(|status| status_matches_scope(status, scope))
        .collect::<Vec<_>>();
    let mut choices = vec![StopChoice::all()];
    choices.extend(
        sorted_statuses
            .iter()
            .map(|status| StopChoice::from_status(status)),
    );
    let selected = MultiSelect::new("Select tunnels to stop:", choices).prompt()?;

    if selected.iter().any(StopChoice::is_all) {
        return Ok(sorted_statuses
            .into_iter()
            .map(|status| status.state.runtime_id.clone())
            .collect());
    }

    Ok(selected
        .into_iter()
        .filter_map(StopChoice::into_tunnel_id)
        .collect())
}

/// 指定 runtime ID のトンネル設定を取得する
fn find_tunnels_by_runtime_ids<'a>(
    config: &'a EffectiveConfig,
    runtime_ids: &[String],
) -> Vec<&'a ResolvedTunnelConfig> {
    let tunnels_by_runtime_id = tunnel_index_by_runtime_id(config);

    runtime_ids
        .iter()
        .filter_map(|runtime_id| tunnels_by_runtime_id.get(runtime_id.as_str()).copied())
        .collect()
}

/// 指定 name のトンネル設定をスコープ優先規則に従って取得する
fn find_tunnels_by_names<'a>(
    config: &'a EffectiveConfig,
    names: &[String],
    scope: Option<ConfigScopeArg>,
) -> Result<Vec<&'a ResolvedTunnelConfig>, ()> {
    names
        .iter()
        .map(|name| find_tunnel_by_name(config, name, scope).ok_or(()))
        .collect()
}

/// 指定 name のトンネル設定をスコープ優先規則に従って取得する
fn find_tunnel_by_name<'a>(
    config: &'a EffectiveConfig,
    name: &str,
    scope: Option<ConfigScopeArg>,
) -> Option<&'a ResolvedTunnelConfig> {
    if let Some(scope) = scope_kind(scope) {
        return config
            .tunnels
            .iter()
            .find(|resolved| resolved.source.kind == scope && resolved.tunnel.name == name);
    }

    config
        .tunnels
        .iter()
        .find(|resolved| {
            resolved.source.kind == ConfigSourceKind::Local && resolved.tunnel.name == name
        })
        .or_else(|| {
            config.tunnels.iter().find(|resolved| {
                resolved.source.kind == ConfigSourceKind::Global && resolved.tunnel.name == name
            })
        })
}

/// 指定 runtime ID のトンネル設定を取得する
fn find_tunnel_by_runtime_id<'a>(
    config: &'a EffectiveConfig,
    runtime_id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .find(|resolved| runtime_id_for_resolved_tunnel(resolved) == runtime_id)
}

/// 統合済みトンネル設定を runtime ID から参照する索引を生成する
fn tunnel_index_by_runtime_id(config: &EffectiveConfig) -> HashMap<String, &ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .map(|resolved| (runtime_id_for_resolved_tunnel(resolved), resolved))
        .collect()
}

/// 指定 name のトンネル状態をスコープ優先規則に従って取得する
fn find_statuses_by_names<'a>(
    statuses: &'a [TunnelRuntimeStatus],
    names: &[String],
    scope: Option<ConfigScopeArg>,
) -> Result<Vec<&'a TunnelRuntimeStatus>, ()> {
    names
        .iter()
        .map(|name| find_status_by_name(statuses, name, scope).ok_or(()))
        .collect()
}

/// 指定 name のトンネル状態をスコープ優先規則に従って取得する
fn find_status_by_name<'a>(
    statuses: &'a [TunnelRuntimeStatus],
    name: &str,
    scope: Option<ConfigScopeArg>,
) -> Option<&'a TunnelRuntimeStatus> {
    if let Some(scope) = scope_kind(scope) {
        return statuses
            .iter()
            .find(|status| status.state.source_kind == scope && status.state.name == name);
    }

    statuses
        .iter()
        .find(|status| {
            status.state.source_kind == ConfigSourceKind::Local && status.state.name == name
        })
        .or_else(|| {
            statuses.iter().find(|status| {
                status.state.source_kind == ConfigSourceKind::Global && status.state.name == name
            })
        })
}

/// トンネル状態を runtime ID から参照する索引を生成する
fn status_index_by_runtime_id(
    statuses: &[TunnelRuntimeStatus],
) -> HashMap<&str, &TunnelRuntimeStatus> {
    statuses
        .iter()
        .map(|status| (status.state.runtime_id.as_str(), status))
        .collect()
}

/// ID 昇順のトンネル状態一覧を生成する
fn sorted_statuses_by_id(statuses: &[TunnelRuntimeStatus]) -> Vec<&TunnelRuntimeStatus> {
    let mut statuses = statuses.iter().collect::<Vec<_>>();
    statuses.sort_by(|left, right| left.state.name.cmp(&right.state.name));

    statuses
}

/// 指定スコープがある場合にトンネル状態の対象可否を判定する
fn status_matches_scope(status: &TunnelRuntimeStatus, scope: Option<ConfigScopeArg>) -> bool {
    scope_kind(scope).is_none_or(|scope| status.state.source_kind == scope)
}

/// stale なトンネルの runtime ID を取得する
fn stale_runtime_ids(
    statuses: &[TunnelRuntimeStatus],
    scope: Option<ConfigScopeArg>,
) -> Vec<String> {
    statuses
        .iter()
        .filter(|status| status.process_state == ProcessState::Stale)
        .filter(|status| status_matches_scope(status, scope))
        .map(|status| status.state.runtime_id.clone())
        .collect()
}

/// watch 対象の stale なトンネル runtime ID を取得する
fn watched_stale_runtime_ids(
    statuses: &[TunnelRuntimeStatus],
    runtime_ids: &[String],
    scope: Option<ConfigScopeArg>,
) -> Vec<String> {
    let requested_runtime_ids = runtime_id_set(runtime_ids);

    statuses
        .iter()
        .filter(|status| status.process_state == ProcessState::Stale)
        .filter(|status| status_matches_scope(status, scope))
        .filter(|status| {
            requested_runtime_ids
                .as_ref()
                .is_none_or(|runtime_ids| runtime_ids.contains(status.state.runtime_id.as_str()))
        })
        .map(|status| status.state.runtime_id.clone())
        .collect()
}

/// watch で検出した stale なトンネルを再起動する
fn recover_watch_stale_tunnels(
    config: &EffectiveConfig,
    state_path: &Path,
    runtime_ids: &[String],
) -> Result<(), CliError> {
    for runtime_id in runtime_ids {
        let Some(tunnel) = find_tunnel_by_runtime_id(config, runtime_id) else {
            eprintln!(
                "{}",
                red(
                    &format!("Configured tunnel not found for stale state: {runtime_id}"),
                    OutputStream::Stderr
                )
            );
            continue;
        };

        match start_tunnel(tunnel, state_path) {
            Ok(started) => print_started_tunnel(&started),
            Err(error) => {
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }

    Ok(())
}

/// 未知の name を表示する
fn print_unknown_names(config: &EffectiveConfig, names: &[String], scope: Option<ConfigScopeArg>) {
    for name in names {
        if find_tunnel_by_name(config, name, scope).is_none() {
            eprintln!(
                "{}",
                red(
                    &format!("Unknown tunnel name: {name}"),
                    OutputStream::Stderr
                )
            );
        }
    }
}

/// 追跡されていない name を表示する
fn print_untracked_names(
    statuses: &[TunnelRuntimeStatus],
    names: &[String],
    scope: Option<ConfigScopeArg>,
) {
    for name in names {
        if find_status_by_name(statuses, name, scope).is_none() {
            eprintln!(
                "{}",
                red(
                    &format!("Tunnel is not tracked: {name}"),
                    OutputStream::Stderr
                )
            );
        }
    }
}

/// watch の開始を表示する
fn print_watch_started(ids: &[String], interval_seconds: u64) {
    let target = if ids.is_empty() {
        "tracked tunnels".to_owned()
    } else {
        format!("tracked tunnels: {}", ids.join(", "))
    };
    let message = format!("Watching {target} every {interval_seconds}s. Press Ctrl-C to stop.");

    println!("{}", green(&message, OutputStream::Stdout));
}

/// SSH コマンド引数をシェル表示用の文字列へ変換する
fn format_ssh_command(args: &[String]) -> String {
    std::iter::once("ssh".to_owned())
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// シェル上で読みやすい単一引数表現へ変換する
fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "@%_+=:,./-".contains(character))
    {
        return value.to_owned();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

/// start の dry-run 結果を表示する
fn print_start_dry_run(tunnels: &[&ResolvedTunnelConfig], state_path: &Path) {
    println!("Dry run: no ssh process will be started and state file will not be written.");
    println!("State file: {}", format_path_for_display(state_path));

    for tunnel in tunnels {
        print_start_dry_run_tunnel(tunnel);
    }
}

/// start dry-run のトンネル単位の結果を表示する
fn print_start_dry_run_tunnel(resolved: &ResolvedTunnelConfig) {
    let tunnel = &resolved.tunnel;
    let local = format_local_endpoint(tunnel);
    let remote = format_remote_endpoint(tunnel);
    let ssh = format_ssh_endpoint(tunnel);
    let command = format_ssh_command(&build_ssh_command_args(resolved));

    println!("Would start tunnel: {}", tunnel.name);
    println!("  Local: {local}");
    println!("  Remote: {remote}");
    println!("  SSH: {ssh}");
    println!("  Command: {command}");
}

/// stop の dry-run 結果を表示する
fn print_stop_dry_run(
    statuses: &[TunnelRuntimeStatus],
    runtime_ids: &[String],
    state_path: &Path,
) -> ExitCode {
    println!("Dry run: no process will be stopped and state file will not be modified.");
    println!("State file: {}", format_path_for_display(state_path));

    let statuses_by_runtime_id = status_index_by_runtime_id(statuses);
    let mut failed = false;

    for runtime_id in runtime_ids {
        let Some(status) = statuses_by_runtime_id.get(runtime_id.as_str()).copied() else {
            failed = true;
            eprintln!(
                "{}",
                red(
                    &format!("Tunnel is not tracked: {runtime_id}"),
                    OutputStream::Stderr
                )
            );
            continue;
        };

        print_stop_dry_run_tunnel(status);
    }

    exit_code_from_failure(failed)
}

/// runtime ID 一覧の一致判定用集合を生成する
fn runtime_id_set(runtime_ids: &[String]) -> Option<HashSet<&str>> {
    if runtime_ids.is_empty() {
        None
    } else {
        Some(runtime_ids.iter().map(String::as_str).collect())
    }
}

/// stop dry-run のトンネル単位の結果を表示する
fn print_stop_dry_run_tunnel(status: &TunnelRuntimeStatus) {
    match status.process_state {
        ProcessState::Running => {
            println!(
                "Would stop tunnel: {} (pid: {})",
                status.state.name, status.state.pid
            );
            println!("Would remove state entry: {}", status.state.name);
        }
        ProcessState::Stale => {
            println!(
                "Would remove stale tunnel state: {} (pid: {})",
                status.state.name, status.state.pid
            );
        }
    }
}

/// 起動したトンネルを表示する
fn print_started_tunnel(started: &StartedTunnel) {
    println!(
        "{}",
        green(
            &format!(
                "Started tunnel: {} (pid: {}, local: {}:{})",
                started.state.name,
                started.state.pid,
                started.state.local_host,
                started.state.local_port
            ),
            OutputStream::Stdout
        )
    );
}

/// 停止したトンネルを表示する
fn print_stopped_tunnel(stopped: &StoppedTunnel) {
    let message = match stopped.previous_state {
        ProcessState::Running => {
            format!(
                "Stopped tunnel: {} (pid: {})",
                stopped.state.name, stopped.state.pid
            )
        }
        ProcessState::Stale => {
            format!(
                "Removed stale tunnel state: {} (pid: {})",
                stopped.state.name, stopped.state.pid
            )
        }
    };

    println!("{}", green(&message, OutputStream::Stdout));
}

/// 状態一覧の見出しを表示する
fn print_status_header(widths: StatusColumnWidths) {
    println!(
        "{} {} {} {} {} SOURCE STARTED",
        pad_display_width("NAME", widths.id),
        pad_display_width("LOCAL", widths.local),
        pad_display_width("REMOTE", widths.remote),
        pad_display_width("PID", widths.pid),
        pad_display_width("STATE", widths.state)
    );
}

/// 状態一覧の 1 行を表示する
fn print_status_row(row: &StatusRow<'_>, widths: StatusColumnWidths) {
    println!(
        "{} {} {} {} {} {} {}",
        pad_display_width_with_visible_width(row.id, row.id_width, widths.id),
        pad_display_width_with_visible_width(&row.local, row.local_width, widths.local),
        pad_display_width_with_visible_width(&row.remote, row.remote_width, widths.remote),
        pad_display_width_with_visible_width(&row.pid, row.pid_width, widths.pid),
        pad_display_width_with_visible_width(
            &row.process_state,
            row.process_state_width,
            widths.state,
        ),
        row.source,
        row.started
    );
}

/// status 表示用の行を生成する
fn status_rows<'a>(statuses: &[&'a TunnelRuntimeStatus], now: u64) -> Vec<StatusRow<'a>> {
    statuses
        .iter()
        .map(|status| StatusRow::from_status(status, now))
        .collect()
}

/// status 表示用に事前整形した 1 行を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusRow<'a> {
    id: &'a str,
    id_width: usize,
    local: String,
    local_width: usize,
    remote: String,
    remote_width: usize,
    pid: String,
    pid_width: usize,
    process_state: String,
    process_state_width: usize,
    source: ConfigSourceKind,
    started: String,
}

impl<'a> StatusRow<'a> {
    /// トンネル実行状態から status 表示行を生成する
    fn from_status(status: &'a TunnelRuntimeStatus, now: u64) -> Self {
        let state = &status.state;
        let local = format_status_local_endpoint(state);
        let remote = format_status_remote_endpoint(state);
        let pid = state.pid.to_string();
        let process_state = process_state_label(status.process_state, OutputStream::Stdout);
        let process_state_width = display_width(process_state_plain_label(status.process_state));
        let started = relative_time_label(state.started_at_unix_seconds, now);

        Self {
            id: state.name.as_str(),
            id_width: display_width(&state.name),
            local_width: display_width(&local),
            local,
            remote_width: display_width(&remote),
            remote,
            pid_width: display_width(&pid),
            pid,
            process_state_width,
            process_state,
            source: state.source_kind,
            started,
        }
    }
}

/// status 表示の列幅を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusColumnWidths {
    id: usize,
    local: usize,
    remote: usize,
    pid: usize,
    state: usize,
}

impl StatusColumnWidths {
    /// 最小表示幅を初期値として列幅を初期化する
    fn with_minimums() -> Self {
        Self {
            id: STATUS_ID_MIN_WIDTH,
            local: STATUS_LOCAL_MIN_WIDTH,
            remote: STATUS_REMOTE_MIN_WIDTH,
            pid: STATUS_PID_MIN_WIDTH,
            state: STATUS_STATE_MIN_WIDTH,
        }
    }
}

/// status 表示対象の値から列幅を算出する
fn status_column_widths(rows: &[StatusRow<'_>]) -> StatusColumnWidths {
    rows.iter()
        .fold(StatusColumnWidths::with_minimums(), |widths, row| {
            StatusColumnWidths {
                id: widths.id.max(row.id_width),
                local: widths.local.max(row.local_width),
                remote: widths.remote.max(row.remote_width),
                pid: widths.pid.max(row.pid_width),
                state: widths.state.max(row.process_state_width),
            }
        })
}

/// 状態一覧表示用の local endpoint 文字列を生成する
fn format_status_local_endpoint(state: &TunnelState) -> String {
    format!("{}:{}", state.local_host, state.local_port)
}

/// 状態一覧表示用の remote endpoint 文字列を生成する
fn format_status_remote_endpoint(state: &TunnelState) -> String {
    format!("{}:{}", state.remote_host, state.remote_port)
}

/// 現在時刻を UNIX 秒で取得する
fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// 起動時刻を現在時刻からの相対表示へ変換する
fn relative_time_label(started_at: u64, now: u64) -> String {
    if started_at > now {
        return "in the future".to_owned();
    }

    let elapsed = now - started_at;

    if elapsed < 60 {
        return format!("{elapsed}s ago");
    }

    let minutes = elapsed / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }

    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }

    let days = hours / 24;
    format!("{days}d ago")
}

/// プロセス状態の表示ラベルを取得する
fn process_state_label(process_state: ProcessState, stream: OutputStream) -> String {
    match process_state {
        ProcessState::Running => green(process_state_plain_label(process_state), stream),
        ProcessState::Stale => red(process_state_plain_label(process_state), stream),
    }
}

/// プロセス状態の未装飾ラベルを取得する
fn process_state_plain_label(process_state: ProcessState) -> &'static str {
    match process_state {
        ProcessState::Running => "RUNNING",
        ProcessState::Stale => "STALE",
    }
}

/// JSON 出力を標準出力へ書き込む
fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    serde_json::to_writer_pretty(io::stdout(), value)?;
    println!();

    Ok(())
}

/// list の JSON 出力を表現する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListJson {
    has_config: bool,
    tunnels: Vec<TunnelJson>,
}

impl ListJson {
    /// 設定ファイルがない場合の JSON 出力を生成する
    fn empty() -> Self {
        Self {
            has_config: false,
            tunnels: Vec::new(),
        }
    }

    /// 統合済みトンネル一覧から JSON 出力を生成する
    fn from_tunnels(tunnels: &[&ResolvedTunnelConfig]) -> Self {
        Self {
            has_config: true,
            tunnels: tunnels
                .iter()
                .map(|resolved| TunnelJson::from_resolved_tunnel(resolved))
                .collect(),
        }
    }
}

/// show の JSON 出力を表現する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowJson {
    tunnel: TunnelJson,
}

impl ShowJson {
    /// 統合済みトンネルから JSON 出力を生成する
    fn from_tunnel(resolved: &ResolvedTunnelConfig) -> Self {
        Self {
            tunnel: TunnelJson::from_resolved_tunnel(resolved),
        }
    }
}

/// トンネル設定の JSON 表現を保持する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelJson {
    name: String,
    runtime_id: String,
    description: Option<String>,
    tags: Vec<String>,
    local_host: String,
    local_port: u16,
    local: String,
    remote_host: String,
    remote_port: u16,
    remote: String,
    ssh_user: String,
    ssh_host: String,
    ssh_port: Option<u16>,
    ssh: String,
    identity_file: Option<String>,
    source: String,
    source_path: String,
    timeouts: TimeoutJson,
}

impl TunnelJson {
    /// 統合済みトンネルから JSON 用の値を生成する
    fn from_resolved_tunnel(resolved: &ResolvedTunnelConfig) -> Self {
        let tunnel = &resolved.tunnel;

        Self {
            name: tunnel.name.clone(),
            runtime_id: runtime_id_for_resolved_tunnel(resolved),
            description: tunnel.description.clone(),
            tags: tunnel.tags.clone(),
            local_host: tunnel.effective_local_host().to_owned(),
            local_port: tunnel.local_port,
            local: format_local_endpoint(tunnel),
            remote_host: tunnel.remote_host.clone(),
            remote_port: tunnel.remote_port,
            remote: format_remote_endpoint(tunnel),
            ssh_user: tunnel.ssh_user.clone(),
            ssh_host: tunnel.ssh_host.clone(),
            ssh_port: tunnel.ssh_port,
            ssh: format_ssh_endpoint(tunnel),
            identity_file: tunnel.identity_file.clone(),
            source: resolved.source.kind.to_string(),
            source_path: resolved.source.path.display().to_string(),
            timeouts: TimeoutJson::from_timeouts(resolved.timeouts),
        }
    }
}

/// タイムアウト設定の JSON 表現を保持する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimeoutJson {
    connect_timeout_seconds: u32,
    server_alive_interval_seconds: u32,
    server_alive_count_max: u32,
    start_grace_milliseconds: u64,
}

impl TimeoutJson {
    /// 解決済みタイムアウトから JSON 用の値を生成する
    fn from_timeouts(timeouts: ResolvedTimeoutConfig) -> Self {
        Self {
            connect_timeout_seconds: timeouts.connect_timeout_seconds,
            server_alive_interval_seconds: timeouts.server_alive_interval_seconds,
            server_alive_count_max: timeouts.server_alive_count_max,
            start_grace_milliseconds: timeouts.start_grace_milliseconds,
        }
    }
}

/// start --dry-run の JSON 出力を表現する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartDryRunJson {
    dry_run: bool,
    state_file: String,
    tunnels: Vec<StartDryRunTunnelJson>,
}

impl StartDryRunJson {
    /// 開始予定トンネルから JSON 出力を生成する
    fn from_tunnels(tunnels: &[&ResolvedTunnelConfig], state_path: &Path) -> Self {
        Self {
            dry_run: true,
            state_file: state_path.display().to_string(),
            tunnels: tunnels
                .iter()
                .map(|resolved| StartDryRunTunnelJson::from_resolved_tunnel(resolved))
                .collect(),
        }
    }
}

/// start --dry-run のトンネル単位 JSON 表現を保持する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartDryRunTunnelJson {
    tunnel: TunnelJson,
    command: String,
    args: Vec<String>,
}

impl StartDryRunTunnelJson {
    /// 統合済みトンネルから開始予定の JSON 表現を生成する
    fn from_resolved_tunnel(resolved: &ResolvedTunnelConfig) -> Self {
        let args = build_ssh_command_args(resolved);

        Self {
            tunnel: TunnelJson::from_resolved_tunnel(resolved),
            command: format_ssh_command(&args),
            args,
        }
    }
}

/// status の JSON 出力を表現する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusJson {
    state_file: String,
    statuses: Vec<StatusTunnelJson>,
}

impl StatusJson {
    /// 実行状態一覧から JSON 出力を生成する
    fn from_statuses(state_path: &Path, statuses: &[&TunnelRuntimeStatus], now: u64) -> Self {
        Self {
            state_file: state_path.display().to_string(),
            statuses: statuses
                .iter()
                .map(|status| StatusTunnelJson::from_status(status, now))
                .collect(),
        }
    }
}

/// status のトンネル単位 JSON 表現を保持する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusTunnelJson {
    name: String,
    runtime_id: String,
    pid: u32,
    process_state: String,
    local_host: String,
    local_port: u16,
    local: String,
    remote_host: String,
    remote_port: u16,
    remote: String,
    ssh_user: String,
    ssh_host: String,
    ssh_port: Option<u16>,
    source: String,
    source_path: String,
    started_at_unix_seconds: u64,
    started: String,
}

impl StatusTunnelJson {
    /// 実行状態から JSON 用の値を生成する
    fn from_status(status: &TunnelRuntimeStatus, now: u64) -> Self {
        let state = &status.state;

        Self {
            name: state.name.clone(),
            runtime_id: state.runtime_id.clone(),
            pid: state.pid,
            process_state: process_state_plain_label(status.process_state).to_ascii_lowercase(),
            local_host: state.local_host.clone(),
            local_port: state.local_port,
            local: format_status_local_endpoint(state),
            remote_host: state.remote_host.clone(),
            remote_port: state.remote_port,
            remote: format_status_remote_endpoint(state),
            ssh_user: state.ssh_user.clone(),
            ssh_host: state.ssh_host.clone(),
            ssh_port: state.ssh_port,
            source: state.source_kind.to_string(),
            source_path: state.source_path.display().to_string(),
            started_at_unix_seconds: state.started_at_unix_seconds,
            started: relative_time_label(state.started_at_unix_seconds, now),
        }
    }
}

/// validate の JSON 出力を表現する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationJson {
    has_config: bool,
    is_valid: bool,
    errors: Vec<ValidationIssueJson>,
    warnings: Vec<ValidationIssueJson>,
}

impl ValidationJson {
    /// 検証結果から JSON 出力を生成する
    fn from_report(has_config: bool, report: &ValidationReport) -> Self {
        Self {
            has_config,
            is_valid: has_config && report.is_valid(),
            errors: report
                .errors
                .iter()
                .map(ValidationIssueJson::from_error)
                .collect(),
            warnings: report
                .warnings
                .iter()
                .map(ValidationIssueJson::from_warning)
                .collect(),
        }
    }
}

/// validate の issue JSON 表現を保持する
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationIssueJson {
    source: String,
    path: String,
    tunnel_name: Option<String>,
    message: String,
}

impl ValidationIssueJson {
    /// 検証エラーから JSON 用の値を生成する
    fn from_error(error: &fwd_deck_core::ValidationError) -> Self {
        Self {
            source: error.source.kind.to_string(),
            path: error.source.path.display().to_string(),
            tunnel_name: error.tunnel_name.clone(),
            message: error.message.clone(),
        }
    }

    /// 検証警告から JSON 用の値を生成する
    fn from_warning(warning: &fwd_deck_core::ValidationWarning) -> Self {
        Self {
            source: warning.source.kind.to_string(),
            path: warning.source.path.display().to_string(),
            tunnel_name: warning.tunnel_name.clone(),
            message: warning.message.clone(),
        }
    }
}

/// 失敗有無から終了コードを取得する
fn exit_code_from_failure(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// 設定検証の結果を表示する
fn validate_command(config: &EffectiveConfig, json: bool) -> Result<ExitCode, CliError> {
    if json {
        let report = validate_config(config);
        print_json(&ValidationJson::from_report(config.has_sources(), &report))?;
        return Ok(if config.has_sources() && report.is_valid() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }

    Ok(print_validation(config))
}

/// 設定検証の結果を表示する
fn print_validation(config: &EffectiveConfig) -> ExitCode {
    if !config.has_sources() {
        eprintln!(
            "{}",
            red("No configuration files were found.", OutputStream::Stderr)
        );
        return ExitCode::FAILURE;
    }

    let report = validate_config(config);

    if report.is_valid() {
        if report.has_warnings() {
            println!(
                "{}",
                yellow(
                    "Configuration is valid with warnings.",
                    OutputStream::Stdout
                )
            );
            print_validation_warnings(&report, OutputStream::Stdout);
        } else {
            println!("{}", green("Configuration is valid.", OutputStream::Stdout));
        }
        return ExitCode::SUCCESS;
    }

    print_validation_errors(&report);
    print_validation_warnings(&report, OutputStream::Stderr);
    ExitCode::FAILURE
}

/// 設定検証エラーを表示する
fn print_validation_errors(report: &ValidationReport) {
    eprintln!("{}", red("Configuration has errors.", OutputStream::Stderr));

    for error in &report.errors {
        let tunnel = error
            .tunnel_name
            .as_ref()
            .map_or(String::from("-"), ToString::to_string);
        let message = format!(
            "- [{}] {} ({}) {}",
            error.source.kind,
            tunnel,
            format_path_for_display(&error.source.path),
            error.message
        );
        eprintln!("{}", red(&message, OutputStream::Stderr));
    }
}

/// 設定検証警告を表示する
fn print_validation_warnings(report: &ValidationReport, stream: OutputStream) {
    if !report.has_warnings() {
        return;
    }

    print_line(&yellow("Configuration has warnings.", stream), stream);

    for warning in &report.warnings {
        let tunnel = warning
            .tunnel_name
            .as_ref()
            .map_or(String::from("-"), ToString::to_string);
        let message = format!(
            "- [{}] {} ({}) {}",
            warning.source.kind,
            tunnel,
            format_path_for_display(&warning.source.path),
            warning.message
        );
        print_line(&yellow(&message, stream), stream);
    }
}

/// 設定スコープの選択肢を表現する
#[derive(Debug, Clone)]
struct ScopeChoice {
    kind: ConfigSourceKind,
    label: String,
}

impl ScopeChoice {
    /// 設定スコープから選択肢を生成する
    fn new(kind: ConfigSourceKind, label: String) -> Self {
        Self { kind, label }
    }
}

impl Display for ScopeChoice {
    /// 選択肢の表示文字列を出力する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// 編集対象の選択肢を表現する
#[derive(Debug, Clone)]
struct EditTargetChoice {
    target: ConfigEditTarget,
    label: String,
}

impl EditTargetChoice {
    /// 編集対象から選択肢を生成する
    fn new(target: ConfigEditTarget) -> Self {
        let label = format!(
            "{} ({})",
            target.scope,
            format_path_for_display(&target.path)
        );

        Self { target, label }
    }
}

impl Display for EditTargetChoice {
    /// 選択肢の表示文字列を出力する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// 削除対象の選択肢を表現する
#[derive(Debug, Clone)]
struct RemoveChoice {
    id: String,
    label: String,
}

impl RemoveChoice {
    /// トンネル設定から削除対象の選択肢を生成する
    fn from_tunnel(tunnel: &TunnelConfig) -> Self {
        Self {
            id: tunnel.name.clone(),
            label: format!(
                "{}  {}:{} -> {}:{}",
                tunnel.name,
                tunnel.effective_local_host(),
                tunnel.local_port,
                tunnel.remote_host,
                tunnel.remote_port
            ),
        }
    }
}

impl Display for RemoveChoice {
    /// 選択肢の表示文字列を出力する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// 開始対象の選択肢を表現する
#[derive(Debug, Clone)]
struct StartChoice {
    id: String,
    label: String,
}

impl StartChoice {
    /// 統合済みトンネル設定から選択肢を生成する
    fn from_resolved_tunnel(resolved: &ResolvedTunnelConfig) -> Self {
        let tunnel = &resolved.tunnel;

        Self {
            id: runtime_id_for_resolved_tunnel(resolved),
            label: format!(
                "{}  {}:{} -> {}:{} ({})",
                tunnel.name,
                tunnel.effective_local_host(),
                tunnel.local_port,
                tunnel.remote_host,
                tunnel.remote_port,
                resolved.source.kind
            ),
        }
    }
}

impl Display for StartChoice {
    /// 選択肢の表示文字列を出力する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// 停止対象の選択肢を表現する
#[derive(Debug, Clone)]
struct StopChoice {
    kind: StopChoiceKind,
    label: String,
}

impl StopChoice {
    /// すべて停止する選択肢を生成する
    fn all() -> Self {
        Self {
            kind: StopChoiceKind::All,
            label: "All tracked tunnels".to_owned(),
        }
    }

    /// 起動状態から選択肢を生成する
    fn from_status(status: &TunnelRuntimeStatus) -> Self {
        let state = &status.state;
        let process_state = match status.process_state {
            ProcessState::Running => "RUNNING",
            ProcessState::Stale => "STALE",
        };

        Self {
            kind: StopChoiceKind::Tunnel(state.runtime_id.clone()),
            label: format!(
                "{}  {}:{} ({}, pid: {}, {})",
                state.name,
                state.local_host,
                state.local_port,
                process_state,
                state.pid,
                state.source_kind
            ),
        }
    }

    /// すべて停止する選択肢かを判定する
    fn is_all(&self) -> bool {
        matches!(self.kind, StopChoiceKind::All)
    }

    /// トンネル runtime ID を取り出す
    fn into_tunnel_id(self) -> Option<String> {
        match self.kind {
            StopChoiceKind::All => None,
            StopChoiceKind::Tunnel(id) => Some(id),
        }
    }
}

impl Display for StopChoice {
    /// 選択肢の表示文字列を出力する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// 停止対象の選択肢種別を表現する
#[derive(Debug, Clone)]
enum StopChoiceKind {
    All,
    Tunnel(String),
}

/// 出力先を表現する
#[derive(Debug, Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

/// 成功時のメッセージを緑で装飾する
fn green(message: &str, stream: OutputStream) -> String {
    colorize(message, "32", stream)
}

/// 失敗時のメッセージを赤で装飾する
fn red(message: &str, stream: OutputStream) -> String {
    colorize(message, "31", stream)
}

/// 注意時のメッセージを黄色で装飾する
fn yellow(message: &str, stream: OutputStream) -> String {
    colorize(message, "33", stream)
}

/// 指定された出力先へ 1 行表示する
fn print_line(message: &str, stream: OutputStream) {
    match stream {
        OutputStream::Stdout => println!("{message}"),
        OutputStream::Stderr => eprintln!("{message}"),
    }
}

/// TTY に出力する場合だけ ANSI color を付与する
fn colorize(message: &str, color_code: &str, stream: OutputStream) -> String {
    if !is_terminal(stream) {
        return message.to_owned();
    }

    format!("\x1b[{color_code}m{message}\x1b[0m")
}

/// 出力先が端末かを判定する
fn is_terminal(stream: OutputStream) -> bool {
    match stream {
        OutputStream::Stdout => io::stdout().is_terminal(),
        OutputStream::Stderr => io::stderr().is_terminal(),
    }
}

#[cfg(test)]
mod tests {
    use fwd_deck_core::{ConfigSource, TunnelState, config::LoadedConfigFile};

    use super::*;

    /// 既存トンネルと同じ ID が検出されることを検証する
    #[test]
    fn find_tunnel_id_conflict_detects_existing_tunnel() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_tunnel_id_conflict(&config, ConfigSourceKind::Local, "db");

        assert_eq!(
            conflict.map(|resolved| resolved.tunnel.name.as_str()),
            Some("db")
        );
    }

    /// 未使用の ID は重複扱いしないことを検証する
    #[test]
    fn find_tunnel_id_conflict_returns_none_for_new_tunnel() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_tunnel_id_conflict(&config, ConfigSourceKind::Local, "cache");

        assert!(conflict.is_none());
    }

    /// 同じ local_port を使う別 ID のトンネルが検出されることを検証する
    #[test]
    fn find_local_port_conflict_detects_other_tunnel() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_local_port_conflict(&config, ConfigSourceKind::Local, "cache", 15432);

        assert_eq!(
            conflict.map(|resolved| resolved.tunnel.name.as_str()),
            Some("db")
        );
    }

    /// 同一 ID の既存トンネルは上書き候補として重複扱いしないことを検証する
    #[test]
    fn find_local_port_conflict_ignores_same_tunnel_id() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_local_port_conflict(&config, ConfigSourceKind::Local, "db", 15432);

        assert!(conflict.is_none());
    }

    /// watch 対象未指定時に stale なトンネルがすべて対象になることを検証する
    #[test]
    fn watched_stale_tunnel_ids_returns_all_stale_tunnels_without_filter() {
        let statuses = vec![
            runtime_status("db", ProcessState::Stale),
            runtime_status("cache", ProcessState::Running),
            runtime_status("search", ProcessState::Stale),
        ];

        let ids = watched_stale_runtime_ids(&statuses, &[], None);

        assert_eq!(ids, vec![runtime_id("db"), runtime_id("search")]);
    }

    /// watch 対象指定時に指定 ID の stale なトンネルだけが対象になることを検証する
    #[test]
    fn watched_stale_tunnel_ids_filters_by_requested_ids() {
        let statuses = vec![
            runtime_status("db", ProcessState::Stale),
            runtime_status("cache", ProcessState::Stale),
            runtime_status("search", ProcessState::Running),
        ];

        let ids = watched_stale_runtime_ids(
            &statuses,
            &[runtime_id("cache"), runtime_id("search")],
            None,
        );

        assert_eq!(ids, vec![runtime_id("cache")]);
    }

    /// トンネル状態一覧が ID 昇順に整列されることを検証する
    #[test]
    fn sorted_statuses_by_id_sorts_statuses_by_id() {
        let statuses = vec![
            runtime_status("prod-db", ProcessState::Running),
            runtime_status("dev-db", ProcessState::Running),
        ];

        let sorted = sorted_statuses_by_id(&statuses);

        assert_eq!(status_ids(&sorted), vec!["dev-db", "prod-db"]);
    }

    /// シェル表示用コマンドが安全な引数をそのまま表示することを検証する
    #[test]
    fn shell_quote_keeps_safe_arguments_unquoted() {
        let quoted = shell_quote("127.0.0.1:15432:db.internal:5432");

        assert_eq!(quoted, "127.0.0.1:15432:db.internal:5432");
    }

    /// シェル表示用コマンドが空白やシングルクォートを含む引数を quote することを検証する
    #[test]
    fn shell_quote_quotes_arguments_with_spaces_and_single_quotes() {
        let quoted = shell_quote("/tmp/key file's name");

        assert_eq!(quoted, "'/tmp/key file'\\''s name'");
    }

    /// アプリ起動用の open 引数が Workspace パスを含むことを検証する
    #[test]
    fn app_open_command_args_include_workspace_path() {
        let args = app_open_command_args("/tmp/my-workspace");

        assert_eq!(
            args,
            vec![
                "-n",
                "-b",
                FWD_DECK_APP_BUNDLE_IDENTIFIER,
                "--args",
                FWD_DECK_APP_OPEN_WORKSPACE_ARG,
                "/tmp/my-workspace"
            ]
        );
    }

    /// アプリ未インストール相当の open 失敗が専用エラーへ変換されることを検証する
    #[test]
    fn app_launch_error_maps_missing_app_output() {
        let error = app_launch_error_from_output(
            Some(1),
            "LSOpenURLsWithRole() failed with error -10814 for the file /tmp.",
        );

        assert!(matches!(error, CliError::AppNotInstalled));
    }

    /// bundle identifier 解決失敗の open 出力が専用エラーへ変換されることを検証する
    #[test]
    fn app_launch_error_maps_bundle_identifier_lookup_failure() {
        let error = app_launch_error_from_output(
            Some(1),
            "LSCopyApplicationURLsForBundleIdentifier() failed while trying to determine the application with bundle identifier dev.oiekjr.fwddeck.",
        );

        assert!(matches!(error, CliError::AppNotInstalled));
    }

    /// Workspace パスが絶対ディレクトリへ正規化されることを検証する
    #[test]
    fn resolve_open_workspace_path_canonicalizes_directory() {
        let temp_dir = tempfile::TempDir::new().expect("create temporary directory");

        let resolved = resolve_open_workspace_path(Some(temp_dir.path().to_path_buf()))
            .expect("resolve workspace path");

        assert!(resolved.is_absolute());
        assert_eq!(
            resolved,
            fs::canonicalize(temp_dir.path()).expect("canonicalize temporary directory")
        );
    }

    /// ファイルは Workspace パスとして拒否されることを検証する
    #[test]
    fn resolve_open_workspace_path_rejects_file() {
        let temp_dir = tempfile::TempDir::new().expect("create temporary directory");
        let file_path = temp_dir.path().join("fwd-deck.toml");
        fs::write(&file_path, "").expect("write file");

        let result = resolve_open_workspace_path(Some(file_path));

        assert!(matches!(
            result,
            Err(CliError::WorkspaceNotDirectory { .. })
        ));
    }

    /// 表示幅の広い文字を含む値が指定幅まで補完されることを検証する
    #[test]
    fn pad_display_width_handles_wide_characters() {
        let value = "prod-ポリゴンDB";

        let padded = pad_display_width(value, 24);

        assert!(padded.starts_with(value));
        assert_eq!(UnicodeWidthStr::width(padded.as_str()), 24);
    }

    /// 指定幅より長い値は省略されないことを検証する
    #[test]
    fn pad_display_width_keeps_long_values() {
        let value = "japandb-as.cluster-clpmwhbh0sfa.ap-northeast-1.rds.amazonaws.com:5432";

        let padded = pad_display_width(value, 24);

        assert_eq!(padded, value);
    }

    /// list 表示の列幅が最長 ID に合わせて拡張されることを検証する
    #[test]
    fn list_column_widths_expands_id_to_longest_value() {
        let config = effective_config_with_tunnels(vec![
            tunnel("db", 15432),
            tunnel("prod-登記情報管理システム", 15433),
        ]);
        let tunnels = config.tunnels.iter().collect::<Vec<_>>();

        let rows = list_rows(&tunnels, false);
        let widths = list_column_widths(&rows);

        assert_eq!(widths.id, display_width("prod-登記情報管理システム"));
    }

    /// status 表示の列幅が最長 ID に合わせて拡張されることを検証する
    #[test]
    fn status_column_widths_expands_id_to_longest_value() {
        let statuses = [
            runtime_status("db", ProcessState::Running),
            runtime_status("prod-登記情報管理システム", ProcessState::Running),
        ];
        let statuses = statuses.iter().collect::<Vec<_>>();

        let rows = status_rows(&statuses, 1_700_000_000);
        let widths = status_column_widths(&rows);

        assert_eq!(widths.id, display_width("prod-登記情報管理システム"));
    }

    /// 装飾付き文字列が可視幅に基づいて補完されることを検証する
    #[test]
    fn pad_display_width_with_visible_width_ignores_decoration_width() {
        let value = "\x1b[32mRUNNING\x1b[0m";

        let padded = pad_display_width_with_visible_width(value, display_width("RUNNING"), 10);

        assert!(padded.starts_with(value));
        assert!(padded.ends_with("   "));
    }

    /// list 表示では remote host を省略して remote port を保持することを検証する
    #[test]
    fn format_list_remote_endpoint_truncates_host_and_keeps_port() {
        let mut tunnel = tunnel("db", 15432);
        tunnel.remote_host =
            "japandb-as.cluster-clpmwhbh0sfa.ap-northeast-1.rds.amazonaws.com".to_owned();

        let remote = format_list_remote_endpoint(&tunnel, false);
        let Some((host, port)) = remote.rsplit_once(':') else {
            panic!("remote endpoint should include a port");
        };

        assert_eq!(display_width(host), LIST_REMOTE_HOST_MAX_WIDTH);
        assert!(host.ends_with(TRUNCATION_MARKER));
        assert_eq!(port, "5432");
    }

    /// wide 指定では remote host を省略しないことを検証する
    #[test]
    fn format_list_remote_endpoint_keeps_full_host_when_wide() {
        let mut tunnel = tunnel("db", 15432);
        tunnel.remote_host =
            "japandb-as.cluster-clpmwhbh0sfa.ap-northeast-1.rds.amazonaws.com".to_owned();

        let remote = format_list_remote_endpoint(&tunnel, true);

        assert_eq!(remote, format_remote_endpoint(&tunnel));
    }

    /// list query が ID に対して大文字小文字を区別せず一致することを検証する
    #[test]
    fn tunnel_matches_list_query_matches_id_case_insensitively() {
        let tunnel = tunnel("Dev-DB", 15432);

        assert!(tunnel_matches_list_query(&tunnel, "dev"));
    }

    /// list query が description に対して大文字小文字を区別せず一致することを検証する
    #[test]
    fn tunnel_matches_list_query_matches_description_case_insensitively() {
        let mut tunnel = tunnel("db", 15432);
        tunnel.description = Some("Development database".to_owned());

        assert!(tunnel_matches_list_query(&tunnel, "database"));
    }

    /// list query と tag 指定が AND 条件で絞り込まれることを検証する
    #[test]
    fn select_tunnels_for_list_matches_tags_and_query() {
        let mut dev_db = tunnel("dev-db", 15432);
        dev_db.tags = vec!["dev".to_owned()];
        let mut prod_db = tunnel("prod-db", 25432);
        prod_db.tags = vec!["prod".to_owned()];
        let config = effective_config_with_tunnels(vec![dev_db, prod_db]);

        let tunnels = select_tunnels_for_list(&config, &["dev".to_owned()], Some("db"));

        assert_eq!(tunnels.len(), 1);
        assert_eq!(tunnels[0].tunnel.name, "dev-db");
    }

    /// list 選択結果が ID 昇順に整列されることを検証する
    #[test]
    fn select_tunnels_for_list_sorts_tunnels_by_id() {
        let config =
            effective_config_with_tunnels(vec![tunnel("prod-db", 25432), tunnel("dev-db", 15432)]);

        let tunnels = select_tunnels_for_list(&config, &[], None);

        assert_eq!(tunnel_ids(&tunnels), vec!["dev-db", "prod-db"]);
    }

    /// show 対象の ID が完全一致で取得されることを検証する
    #[test]
    fn find_tunnel_by_id_returns_exact_match() {
        let config = effective_config_with_tunnels(vec![tunnel("dev-db", 15432)]);

        let tunnel = find_tunnel_by_name(&config, "dev-db", None);

        assert_eq!(
            tunnel.map(|resolved| resolved.tunnel.name.as_str()),
            Some("dev-db")
        );
    }

    /// show 対象の ID が部分一致では取得されないことを検証する
    #[test]
    fn find_tunnel_by_id_does_not_return_partial_match() {
        let config = effective_config_with_tunnels(vec![tunnel("dev-db", 15432)]);

        let tunnel = find_tunnel_by_name(&config, "dev", None);

        assert!(tunnel.is_none());
    }

    /// テスト用の統合済み設定を生成する
    fn effective_config_with_tunnels(tunnels: Vec<TunnelConfig>) -> EffectiveConfig {
        let source = ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml"));
        let resolved_tunnels = tunnels
            .iter()
            .cloned()
            .map(|tunnel| ResolvedTunnelConfig::new(source.clone(), tunnel))
            .collect::<Vec<_>>();

        EffectiveConfig::new(
            vec![LoadedConfigFile::new(source, tunnels)],
            resolved_tunnels,
        )
    }

    /// テスト用のトンネル ID 一覧を取得する
    fn tunnel_ids<'a>(tunnels: &[&'a ResolvedTunnelConfig]) -> Vec<&'a str> {
        tunnels
            .iter()
            .map(|resolved| resolved.tunnel.name.as_str())
            .collect()
    }

    /// テスト用の状態 ID 一覧を取得する
    fn status_ids<'a>(statuses: &[&'a TunnelRuntimeStatus]) -> Vec<&'a str> {
        statuses
            .iter()
            .map(|status| status.state.name.as_str())
            .collect()
    }

    /// テスト用のトンネル実行状態を生成する
    fn runtime_status(id: &str, process_state: ProcessState) -> TunnelRuntimeStatus {
        TunnelRuntimeStatus {
            state: TunnelState {
                runtime_id: runtime_id(id),
                name: id.to_owned(),
                pid: 1000,
                local_host: "127.0.0.1".to_owned(),
                local_port: 15432,
                remote_host: "db.internal".to_owned(),
                remote_port: 5432,
                ssh_user: "user".to_owned(),
                ssh_host: "bastion.example.com".to_owned(),
                ssh_port: None,
                source_kind: ConfigSourceKind::Local,
                source_path: PathBuf::from("fwd-deck.toml"),
                started_at_unix_seconds: 1_700_000_000,
            },
            process_state,
        }
    }

    /// テスト用のトンネル設定を生成する
    fn tunnel(id: &str, local_port: u16) -> TunnelConfig {
        TunnelConfig {
            name: id.to_owned(),
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

    /// テスト用の runtime ID を生成する
    fn runtime_id(name: &str) -> String {
        fwd_deck_core::tunnel_runtime_id(
            ConfigSourceKind::Local,
            &PathBuf::from("fwd-deck.toml"),
            name,
        )
    }
}
