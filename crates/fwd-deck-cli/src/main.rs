use std::{
    env,
    fmt::{self, Display},
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand};
use fwd_deck_core::{
    ConfigPaths, EffectiveConfig, ProcessState, ResolvedTunnelConfig, StartedTunnel, StoppedTunnel,
    TunnelRuntimeError, TunnelRuntimeStatus, ValidationReport, default_global_config_path,
    default_local_config_path, default_state_file_path, load_effective_config, start_tunnel,
    stop_tunnel, tunnel_statuses, validate_config,
};
use inquire::{InquireError, MultiSelect};
use thiserror::Error;

/// fwd-deck の CLI 引数を表現する
#[derive(Debug, Parser)]
#[command(
    name = "fwd-deck",
    version,
    about = "Operate port forwarding entries defined in configuration files"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Read local configuration from PATH"
    )]
    config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Read global configuration from PATH"
    )]
    global_config: Option<PathBuf>,

    #[arg(long, global = true, help = "Do not read the global configuration")]
    no_global: bool,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Read and write runtime state from PATH"
    )]
    state: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

/// fwd-deck が提供するサブコマンドを表現する
#[derive(Debug, Clone, Subcommand)]
enum Command {
    #[command(about = "List configured tunnels")]
    List,
    #[command(about = "Start configured tunnels")]
    Start {
        #[arg(value_name = "ID", help = "Tunnel IDs to start")]
        ids: Vec<String>,
    },
    #[command(about = "Recover stale tracked tunnels")]
    Recover {
        #[arg(value_name = "ID", help = "Tunnel IDs to recover")]
        ids: Vec<String>,
    },
    #[command(about = "Show tracked tunnel status")]
    Status,
    #[command(about = "Stop tracked tunnels")]
    Stop {
        #[arg(value_name = "ID", help = "Tunnel IDs to stop")]
        ids: Vec<String>,
    },
    #[command(about = "Validate configuration files")]
    Validate,
}

/// CLI 実行時の失敗理由を表現する
#[derive(Debug, Error)]
enum CliError {
    #[error("Failed to get the current directory: {0}")]
    CurrentDir(std::io::Error),
    #[error("Failed to resolve the default state file path because HOME is not set")]
    MissingStateHome,
    #[error(transparent)]
    Config(#[from] fwd_deck_core::ConfigLoadError),
    #[error(transparent)]
    Runtime(#[from] TunnelRuntimeError),
    #[error(transparent)]
    Prompt(#[from] InquireError),
}

/// CLI の実行入口を初期化する
fn main() -> ExitCode {
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
        Command::List => {
            let config = load_config(&cli)?;
            print_list(&config);
            Ok(ExitCode::SUCCESS)
        }
        Command::Start { ids } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            start_command(&config, &state_path, ids.clone())
        }
        Command::Recover { ids } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            recover_command(&config, &state_path, ids.clone())
        }
        Command::Status => {
            let state_path = resolve_state_path(state_path)?;
            status_command(&state_path)
        }
        Command::Stop { ids } => {
            let state_path = resolve_state_path(state_path)?;
            stop_command(&state_path, ids.clone())
        }
        Command::Validate => {
            let config = load_config(&cli)?;
            Ok(print_validation(&config))
        }
    }
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

/// 統合済みトンネル設定の一覧を表示する
fn print_list(config: &EffectiveConfig) {
    if !config.has_sources() {
        println!(
            "{}",
            red("No configuration files were found.", OutputStream::Stdout)
        );
        return;
    }

    if config.tunnels.is_empty() {
        println!("No tunnels are configured.");
        return;
    }

    println!(
        "{:<24} {:<24} {:<32} {:<32} SOURCE",
        "ID", "LOCAL", "REMOTE", "SSH"
    );

    for resolved in &config.tunnels {
        print_tunnel_row(resolved);
    }
}

/// トンネル設定の一覧行を表示する
fn print_tunnel_row(resolved: &ResolvedTunnelConfig) {
    let tunnel = &resolved.tunnel;
    let local = format!("{}:{}", tunnel.effective_local_host(), tunnel.local_port);
    let remote = format!("{}:{}", tunnel.remote_host, tunnel.remote_port);
    let ssh = match tunnel.ssh_port {
        Some(port) => format!("{}@{}:{}", tunnel.ssh_user, tunnel.ssh_host, port),
        None => format!("{}@{}", tunnel.ssh_user, tunnel.ssh_host),
    };

    println!(
        "{:<24} {:<24} {:<32} {:<32} {}",
        tunnel.id, local, remote, ssh, resolved.source.kind
    );
}

/// トンネル開始コマンドを実行する
fn start_command(
    config: &EffectiveConfig,
    state_path: &Path,
    ids: Vec<String>,
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

    let ids = if ids.is_empty() {
        prompt_tunnels_to_start(config)?
    } else {
        ids
    };

    if ids.is_empty() {
        println!("No tunnels were selected.");
        return Ok(ExitCode::SUCCESS);
    }

    let Ok(tunnels) = find_tunnels_by_ids(config, &ids) else {
        print_unknown_ids(config, &ids);
        return Ok(ExitCode::FAILURE);
    };
    let mut failed = false;

    for tunnel in tunnels {
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

/// stale なトンネルを再起動する
fn recover_command(
    config: &EffectiveConfig,
    state_path: &Path,
    ids: Vec<String>,
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

    let recovery_ids = if ids.is_empty() {
        stale_tunnel_ids(&statuses)
    } else {
        ids
    };

    if recovery_ids.is_empty() {
        println!("No stale tunnels to recover.");
        return Ok(ExitCode::SUCCESS);
    }

    let mut failed = false;

    for id in recovery_ids {
        let Some(status) = statuses.iter().find(|status| status.state.id == id) else {
            failed = true;
            eprintln!(
                "{}",
                red(
                    &format!("Tunnel is not tracked: {id}"),
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
                        status.state.id, status.state.pid
                    ),
                    OutputStream::Stdout
                )
            );
            continue;
        }

        let Some(tunnel) = find_tunnel_by_id(config, &id) else {
            failed = true;
            eprintln!(
                "{}",
                red(
                    &format!("Configured tunnel not found for stale state: {id}"),
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

/// トンネル状態表示コマンドを実行する
fn status_command(state_path: &Path) -> Result<ExitCode, CliError> {
    let statuses = tunnel_statuses(state_path)?;

    if statuses.is_empty() {
        println!("No tracked tunnels.");
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "{:<24} {:<24} {:<32} {:<8} {:<10} STARTED",
        "ID", "LOCAL", "REMOTE", "PID", "STATE"
    );

    let now = current_unix_seconds();

    for status in &statuses {
        print_status_row(status, now);
    }

    Ok(ExitCode::SUCCESS)
}

/// トンネル停止コマンドを実行する
fn stop_command(state_path: &Path, ids: Vec<String>) -> Result<ExitCode, CliError> {
    let statuses = tunnel_statuses(state_path)?;

    if statuses.is_empty() && ids.is_empty() {
        println!("No tracked tunnels.");
        return Ok(ExitCode::SUCCESS);
    }

    let ids = if ids.is_empty() {
        prompt_tunnels_to_stop(&statuses)?
    } else {
        ids
    };

    if ids.is_empty() {
        println!("No tunnels were selected.");
        return Ok(ExitCode::SUCCESS);
    }

    let mut failed = false;

    for id in ids {
        match stop_tunnel(&id, state_path) {
            Ok(stopped) => print_stopped_tunnel(&stopped),
            Err(error) => {
                failed = true;
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }

    Ok(exit_code_from_failure(failed))
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

/// 開始対象のトンネルを対話的に選択する
fn prompt_tunnels_to_start(config: &EffectiveConfig) -> Result<Vec<String>, InquireError> {
    let choices = config
        .tunnels
        .iter()
        .map(StartChoice::from_resolved_tunnel)
        .collect::<Vec<_>>();

    if choices.is_empty() {
        return Ok(Vec::new());
    }

    let selected = MultiSelect::new("Select tunnels to start:", choices).prompt()?;

    Ok(selected.into_iter().map(|choice| choice.id).collect())
}

/// 停止対象のトンネルを対話的に選択する
fn prompt_tunnels_to_stop(statuses: &[TunnelRuntimeStatus]) -> Result<Vec<String>, InquireError> {
    let mut choices = vec![StopChoice::all()];
    choices.extend(statuses.iter().map(StopChoice::from_status));
    let selected = MultiSelect::new("Select tunnels to stop:", choices).prompt()?;

    if selected.iter().any(StopChoice::is_all) {
        return Ok(statuses
            .iter()
            .map(|status| status.state.id.clone())
            .collect());
    }

    Ok(selected
        .into_iter()
        .filter_map(StopChoice::into_tunnel_id)
        .collect())
}

/// 指定 ID のトンネル設定を取得する
fn find_tunnels_by_ids<'a>(
    config: &'a EffectiveConfig,
    ids: &[String],
) -> Result<Vec<&'a ResolvedTunnelConfig>, ()> {
    let mut tunnels = Vec::new();

    for id in ids {
        let Some(tunnel) = config.tunnels.iter().find(|tunnel| tunnel.tunnel.id == *id) else {
            return Err(());
        };
        tunnels.push(tunnel);
    }

    Ok(tunnels)
}

/// 指定 ID のトンネル設定を取得する
fn find_tunnel_by_id<'a>(
    config: &'a EffectiveConfig,
    id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config.tunnels.iter().find(|tunnel| tunnel.tunnel.id == id)
}

/// stale なトンネル ID を取得する
fn stale_tunnel_ids(statuses: &[TunnelRuntimeStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|status| status.process_state == ProcessState::Stale)
        .map(|status| status.state.id.clone())
        .collect()
}

/// 未知の ID を表示する
fn print_unknown_ids(config: &EffectiveConfig, ids: &[String]) {
    let known_ids = config
        .tunnels
        .iter()
        .map(|tunnel| tunnel.tunnel.id.as_str())
        .collect::<Vec<_>>();

    for id in ids {
        if !known_ids.contains(&id.as_str()) {
            eprintln!(
                "{}",
                red(&format!("Unknown tunnel id: {id}"), OutputStream::Stderr)
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
                started.state.id,
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
                stopped.state.id, stopped.state.pid
            )
        }
        ProcessState::Stale => {
            format!(
                "Removed stale tunnel state: {} (pid: {})",
                stopped.state.id, stopped.state.pid
            )
        }
    };

    println!("{}", green(&message, OutputStream::Stdout));
}

/// 状態一覧の 1 行を表示する
fn print_status_row(status: &TunnelRuntimeStatus, now: u64) {
    let state = &status.state;
    let local = format!("{}:{}", state.local_host, state.local_port);
    let remote = format!("{}:{}", state.remote_host, state.remote_port);
    let process_state = process_state_label(status.process_state, OutputStream::Stdout);
    let started = relative_time_label(state.started_at_unix_seconds, now);

    println!(
        "{:<24} {:<24} {:<32} {:<8} {:<10} {}",
        state.id, local, remote, state.pid, process_state, started
    );
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
        ProcessState::Running => green("RUNNING", stream),
        ProcessState::Stale => red("STALE", stream),
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
        println!("{}", green("Configuration is valid.", OutputStream::Stdout));
        return ExitCode::SUCCESS;
    }

    print_validation_errors(&report);
    ExitCode::FAILURE
}

/// 設定検証エラーを表示する
fn print_validation_errors(report: &ValidationReport) {
    eprintln!("{}", red("Configuration has errors.", OutputStream::Stderr));

    for error in &report.errors {
        let tunnel = error
            .tunnel_id
            .as_ref()
            .map_or(String::from("-"), ToString::to_string);
        let message = format!(
            "- [{}] {} ({}) {}",
            error.source.kind,
            tunnel,
            error.source.path.display(),
            error.message
        );
        eprintln!("{}", red(&message, OutputStream::Stderr));
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
            id: tunnel.id.clone(),
            label: format!(
                "{}  {}:{} -> {}:{}",
                tunnel.id,
                tunnel.effective_local_host(),
                tunnel.local_port,
                tunnel.remote_host,
                tunnel.remote_port
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
            kind: StopChoiceKind::Tunnel(state.id.clone()),
            label: format!(
                "{}  {}:{} ({}, pid: {})",
                state.id, state.local_host, state.local_port, process_state, state.pid
            ),
        }
    }

    /// すべて停止する選択肢かを判定する
    fn is_all(&self) -> bool {
        matches!(self.kind, StopChoiceKind::All)
    }

    /// トンネル ID を取り出す
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
