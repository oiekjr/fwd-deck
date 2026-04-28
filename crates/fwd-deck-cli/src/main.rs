use std::{
    env,
    fmt::{self, Display},
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand, ValueEnum};
use fwd_deck_core::{
    ConfigEditError, ConfigPaths, ConfigSourceKind, DEFAULT_LOCAL_HOST, EffectiveConfig,
    ProcessState, ResolvedTunnelConfig, StartedTunnel, StoppedTunnel, TunnelConfig,
    TunnelRuntimeError, TunnelRuntimeStatus, ValidationReport, add_tunnel_to_config_file,
    default_global_config_path, default_local_config_path, default_state_file_path,
    filter_tunnels_by_tags, load_effective_config, normalize_tag, read_config_file,
    remove_tunnel_from_config_file, start_tunnel, stop_tunnel, tag_is_valid, tunnel_statuses,
    validate_config,
};
use inquire::{Confirm, InquireError, MultiSelect, Select, Text};
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
    List {
        #[arg(long = "tag", value_name = "TAG", help = "Filter tunnels by tag")]
        tags: Vec<String>,
    },
    #[command(about = "Start configured tunnels")]
    Start {
        #[arg(value_name = "ID", help = "Tunnel IDs to start")]
        ids: Vec<String>,
        #[arg(long = "tag", value_name = "TAG", help = "Start tunnels matching tag")]
        tags: Vec<String>,
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
    #[command(about = "Edit configuration files")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "Validate configuration files")]
    Validate,
}

/// 設定編集サブコマンドを表現する
#[derive(Debug, Clone, Subcommand)]
enum ConfigCommand {
    #[command(about = "Add a tunnel to a configuration file")]
    Add {
        #[arg(long, value_enum, help = "Configuration scope to edit")]
        scope: Option<ConfigScopeArg>,
    },
    #[command(about = "Remove a tunnel from a configuration file")]
    Remove {
        #[arg(long, value_enum, help = "Configuration scope to edit")]
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
    #[error(transparent)]
    Config(#[from] fwd_deck_core::ConfigLoadError),
    #[error(transparent)]
    ConfigEdit(#[from] ConfigEditError),
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
        Command::List { tags } => {
            let config = load_config(&cli)?;
            list_command(&config, tags.clone())
        }
        Command::Start { ids, tags } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            start_command(&config, &state_path, ids.clone(), tags.clone())
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
        Command::Config { command } => {
            let paths = resolve_edit_config_paths(&cli)?;
            config_command(&paths, command.clone())
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

/// 設定編集用の設定ファイル位置を解決する
fn resolve_edit_config_paths(cli: &Cli) -> Result<ConfigPaths, CliError> {
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

/// 設定編集コマンドを実行する
fn config_command(paths: &ConfigPaths, command: ConfigCommand) -> Result<ExitCode, CliError> {
    match command {
        ConfigCommand::Add { scope } => config_add_command(paths, scope),
        ConfigCommand::Remove { scope } => config_remove_command(paths, scope),
    }
}

/// 設定ファイルへトンネルを追加する
fn config_add_command(
    paths: &ConfigPaths,
    scope: Option<ConfigScopeArg>,
) -> Result<ExitCode, CliError> {
    let scope = resolve_config_scope(paths, scope)?;
    let path = config_path_for_scope(paths, scope)?;
    let tunnel = prompt_tunnel_config()?;

    add_tunnel_to_config_file(&path, scope, tunnel)?;
    println!(
        "{}",
        green(
            &format!(
                "Added tunnel to {} configuration: {}",
                scope,
                path.display()
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
                &format!("Configuration file was not found: {}", path.display()),
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    };

    if file.tunnels.is_empty() {
        println!("No tunnels are configured in {}.", path.display());
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

/// トンネル設定一覧コマンドを実行する
fn list_command(config: &EffectiveConfig, tags: Vec<String>) -> Result<ExitCode, CliError> {
    let tags = normalize_cli_tags(&tags)?;

    if !config.has_sources() {
        println!(
            "{}",
            red("No configuration files were found.", OutputStream::Stdout)
        );
        return Ok(ExitCode::SUCCESS);
    }

    let tunnels = select_tunnels_for_tags(config, &tags);

    if tunnels.is_empty() && tags.is_empty() {
        println!("No tunnels are configured.");
        return Ok(ExitCode::SUCCESS);
    }

    if tunnels.is_empty() {
        println!("No tunnels matched tags: {}.", tags.join(", "));
        return Ok(ExitCode::SUCCESS);
    }

    print_list(&tunnels);
    Ok(ExitCode::SUCCESS)
}

/// 統合済みトンネル設定の一覧を表示する
fn print_list(tunnels: &[&ResolvedTunnelConfig]) {
    println!(
        "{:<24} {:<24} {:<32} {:<32} {:<24} SOURCE",
        "ID", "LOCAL", "REMOTE", "SSH", "TAGS"
    );

    for resolved in tunnels {
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
    let tags = format_tag_list(&tunnel.tags);

    println!(
        "{:<24} {:<24} {:<32} {:<32} {:<24} {}",
        tunnel.id, local, remote, ssh, tags, resolved.source.kind
    );
}

/// タグ一覧を表示用文字列へ変換する
fn format_tag_list(tags: &[String]) -> String {
    if tags.is_empty() {
        "-".to_owned()
    } else {
        tags.join(",")
    }
}

/// トンネル開始コマンドを実行する
fn start_command(
    config: &EffectiveConfig,
    state_path: &Path,
    ids: Vec<String>,
    tags: Vec<String>,
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

    if !ids.is_empty() && !tags.is_empty() {
        eprintln!(
            "{}",
            red(
                "Cannot combine tunnel IDs with --tag.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    let tags = normalize_cli_tags(&tags)?;

    let ids = if !tags.is_empty() {
        let tunnels = select_tunnels_for_tags(config, &tags);

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
            .into_iter()
            .map(|tunnel| tunnel.tunnel.id.clone())
            .collect()
    } else if ids.is_empty() {
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
        return config.tunnels.iter().collect();
    }

    filter_tunnels_by_tags(&config.tunnels, tags)
}

/// 編集対象の設定スコープを対話的に選択する
fn prompt_config_scope(paths: &ConfigPaths) -> Result<ConfigSourceKind, InquireError> {
    let mut choices = Vec::new();

    choices.push(ScopeChoice::new(
        ConfigSourceKind::Local,
        format!("local ({})", paths.local.display()),
    ));

    if let Some(global_path) = &paths.global {
        choices.push(ScopeChoice::new(
            ConfigSourceKind::Global,
            format!("global ({})", global_path.display()),
        ));
    }

    let selected = Select::new("Select configuration scope:", choices).prompt()?;
    Ok(selected.kind)
}

/// 追加するトンネル設定を対話的に入力する
fn prompt_tunnel_config() -> Result<TunnelConfig, CliError> {
    let id = prompt_required_text("Tunnel id:")?;
    let description = prompt_optional_text("Description:")?;
    let tags = prompt_tags()?;
    let local_host = Some(prompt_local_host()?);
    let local_port = prompt_port("Local port:", None)?;
    let remote_host = prompt_required_text("Remote host:")?;
    let remote_port = prompt_port("Remote port:", None)?;
    let ssh_user = prompt_required_text("SSH user:")?;
    let ssh_host = prompt_required_text("SSH host:")?;
    let ssh_port = Some(prompt_port("SSH port:", Some(22))?);
    let identity_file = prompt_optional_text("Identity file:")?;

    Ok(TunnelConfig {
        id,
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

/// 設定検証警告を表示する
fn print_validation_warnings(report: &ValidationReport, stream: OutputStream) {
    if !report.has_warnings() {
        return;
    }

    print_line(&yellow("Configuration has warnings.", stream), stream);

    for warning in &report.warnings {
        let tunnel = warning
            .tunnel_id
            .as_ref()
            .map_or(String::from("-"), ToString::to_string);
        let message = format!(
            "- [{}] {} ({}) {}",
            warning.source.kind,
            tunnel,
            warning.source.path.display(),
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
