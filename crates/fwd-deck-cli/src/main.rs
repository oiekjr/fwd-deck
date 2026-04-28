use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::{self, Display},
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::ExitCode,
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
    filter_tunnels_by_tags, load_effective_config, normalize_tag, read_config_file,
    remove_tunnel_from_config_file, start_tunnel, stop_tunnel, tag_is_valid, tunnel_statuses,
    validate_config,
};
use inquire::{Confirm, InquireError, MultiSelect, Select, Text};
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
        #[arg(
            long,
            value_name = "TEXT",
            help = "Filter tunnels by id or description"
        )]
        query: Option<String>,
        #[arg(long, help = "Show full remote hosts without truncation")]
        wide: bool,
    },
    #[command(about = "Show configured tunnel details")]
    Show {
        #[arg(value_name = "ID", help = "Tunnel ID to show")]
        id: String,
    },
    #[command(about = "Start configured tunnels")]
    Start {
        #[arg(value_name = "ID", help = "Tunnel IDs to start")]
        ids: Vec<String>,
        #[arg(long = "tag", value_name = "TAG", help = "Start tunnels matching tag")]
        tags: Vec<String>,
        #[arg(long, help = "Start all configured tunnels")]
        all: bool,
        #[arg(long, help = "Preview start actions without starting ssh")]
        dry_run: bool,
    },
    #[command(about = "Recover stale tracked tunnels")]
    Recover {
        #[arg(value_name = "ID", help = "Tunnel IDs to recover")]
        ids: Vec<String>,
    },
    #[command(about = "Watch tracked tunnels and recover stale tunnels")]
    Watch {
        #[arg(value_name = "ID", help = "Tunnel IDs to watch")]
        ids: Vec<String>,
        #[arg(
            long,
            default_value_t = DEFAULT_WATCH_INTERVAL_SECONDS,
            value_parser = clap::value_parser!(u64).range(1..),
            help = "Watch interval in seconds"
        )]
        interval_seconds: u64,
    },
    #[command(about = "Show tracked tunnel status")]
    Status,
    #[command(about = "Stop tracked tunnels")]
    Stop {
        #[arg(value_name = "ID", help = "Tunnel IDs to stop")]
        ids: Vec<String>,
        #[arg(long, help = "Stop all tracked tunnels")]
        all: bool,
        #[arg(long, help = "Preview stop actions without stopping processes")]
        dry_run: bool,
    },
    #[command(about = "Edit configuration files")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "Generate shell completion script")]
    Completion {
        #[arg(value_enum, help = "Shell to generate completions for")]
        shell: Shell,
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
        Command::List { tags, query, wide } => {
            let config = load_config(&cli)?;
            list_command(&config, tags.clone(), query.clone(), *wide)
        }
        Command::Show { id } => {
            let config = load_config(&cli)?;
            show_command(&config, id)
        }
        Command::Start {
            ids,
            tags,
            all,
            dry_run,
        } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            start_command(
                &config,
                &state_path,
                ids.clone(),
                tags.clone(),
                *all,
                *dry_run,
            )
        }
        Command::Recover { ids } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            recover_command(&config, &state_path, ids.clone())
        }
        Command::Watch {
            ids,
            interval_seconds,
        } => {
            let config = load_config(&cli)?;
            let state_path = resolve_state_path(state_path)?;
            watch_command(&config, &state_path, ids.clone(), *interval_seconds)
        }
        Command::Status => {
            let state_path = resolve_state_path(state_path)?;
            status_command(&state_path)
        }
        Command::Stop { ids, all, dry_run } => {
            let state_path = resolve_state_path(state_path)?;
            stop_command(&state_path, ids.clone(), *all, *dry_run)
        }
        Command::Config { command } => {
            let paths = resolve_config_paths(&cli)?;
            config_command(&paths, command.clone())
        }
        Command::Completion { shell } => completion_command(*shell),
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

/// シェル補完スクリプトを生成する
fn completion_command(shell: Shell) -> Result<ExitCode, CliError> {
    let mut command = Cli::command();
    let binary_name = command.get_name().to_owned();

    clap_complete::generate(shell, &mut command, binary_name, &mut io::stdout());

    Ok(ExitCode::SUCCESS)
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
    let config = load_effective_config(paths)?;
    let tunnel = prompt_tunnel_config(&config)?;

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
fn list_command(
    config: &EffectiveConfig,
    tags: Vec<String>,
    query: Option<String>,
    wide: bool,
) -> Result<ExitCode, CliError> {
    let tags = normalize_cli_tags(&tags)?;
    let query = normalize_list_query(query);

    if !config.has_sources() {
        println!(
            "{}",
            red("No configuration files were found.", OutputStream::Stdout)
        );
        return Ok(ExitCode::SUCCESS);
    }

    let tunnels = select_tunnels_for_list(config, &tags, query.as_deref());

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
fn show_command(config: &EffectiveConfig, id: &str) -> Result<ExitCode, CliError> {
    if !config.has_sources() {
        eprintln!(
            "{}",
            red("No configuration files were found.", OutputStream::Stderr)
        );
        return Ok(ExitCode::FAILURE);
    }

    let Some(resolved) = find_tunnel_by_id(config, id) else {
        eprintln!(
            "{}",
            red(
                &format!("No tunnel matched ID: {id}."),
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    };

    print_tunnel_details(resolved);
    Ok(ExitCode::SUCCESS)
}

/// 統合済みトンネル設定の一覧を表示する
fn print_list(tunnels: &[&ResolvedTunnelConfig], wide: bool) {
    let rows = list_rows(tunnels, wide);
    let widths = list_column_widths(&rows);

    println!(
        "{} {} {} {} {} SOURCE",
        pad_display_width("ID", widths.id),
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
            id: tunnel.id.as_str(),
            id_width: display_width(&tunnel.id),
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

    println!("ID: {}", tunnel.id);
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
        resolved.source.path.display()
    );
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
    ids: Vec<String>,
    tags: Vec<String>,
    all: bool,
    dry_run: bool,
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

    if all && (!ids.is_empty() || !tags.is_empty()) {
        eprintln!(
            "{}",
            red(
                "Cannot combine --all with tunnel IDs or --tag.",
                OutputStream::Stderr
            )
        );
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

    let ids = if all {
        sorted_tunnels_by_id(&config.tunnels)
            .into_iter()
            .map(|tunnel| tunnel.tunnel.id.clone())
            .collect()
    } else if !tags.is_empty() {
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

    if dry_run {
        print_start_dry_run(&tunnels, state_path);
        return Ok(ExitCode::SUCCESS);
    }

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

    let statuses_by_id = status_index_by_id(&statuses);
    let mut failed = false;

    for id in recovery_ids {
        let Some(status) = statuses_by_id.get(id.as_str()).copied() else {
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

/// 追跡中トンネルを監視して stale なトンネルを再起動する
fn watch_command(
    config: &EffectiveConfig,
    state_path: &Path,
    ids: Vec<String>,
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

    if !ids.is_empty() && find_tunnels_by_ids(config, &ids).is_err() {
        print_unknown_ids(config, &ids);
        return Ok(ExitCode::FAILURE);
    }

    print_watch_started(&ids, interval_seconds);
    let interval = Duration::from_secs(interval_seconds);

    loop {
        let statuses = tunnel_statuses(state_path)?;
        let stale_ids = watched_stale_tunnel_ids(&statuses, &ids);

        if !stale_ids.is_empty() {
            recover_watch_stale_tunnels(config, state_path, &stale_ids)?;
        }

        thread::sleep(interval);
    }
}

/// トンネル状態表示コマンドを実行する
fn status_command(state_path: &Path) -> Result<ExitCode, CliError> {
    let statuses = tunnel_statuses(state_path)?;

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
    ids: Vec<String>,
    all: bool,
    dry_run: bool,
) -> Result<ExitCode, CliError> {
    let statuses = tunnel_statuses(state_path)?;

    if all && !ids.is_empty() {
        eprintln!(
            "{}",
            red(
                "Cannot combine --all with tunnel IDs.",
                OutputStream::Stderr
            )
        );
        return Ok(ExitCode::FAILURE);
    }

    if statuses.is_empty() && ids.is_empty() {
        println!("No tracked tunnels.");
        return Ok(ExitCode::SUCCESS);
    }

    let ids = if all {
        sorted_statuses_by_id(&statuses)
            .into_iter()
            .map(|status| status.state.id.clone())
            .collect()
    } else if ids.is_empty() {
        prompt_tunnels_to_stop(&statuses)?
    } else {
        ids
    };

    if ids.is_empty() {
        println!("No tunnels were selected.");
        return Ok(ExitCode::SUCCESS);
    }

    if dry_run {
        return Ok(print_stop_dry_run(&statuses, &ids, state_path));
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
        return sorted_tunnels_by_id(&config.tunnels);
    }

    sorted_tunnels_by_id(filter_tunnels_by_tags(&config.tunnels, tags))
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
    tunnels.sort_by(|left, right| left.tunnel.id.cmp(&right.tunnel.id));

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
    ascii_case_insensitive_contains(&tunnel.id, query)
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
fn prompt_tunnel_config(config: &EffectiveConfig) -> Result<TunnelConfig, CliError> {
    let id = prompt_available_tunnel_id(config)?;
    let description = prompt_optional_text("Description:")?;
    let tags = prompt_tags()?;
    let local_host = Some(prompt_local_host()?);
    let local_port = prompt_available_local_port(config, &id)?;
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
        timeouts: TimeoutConfig::default(),
    })
}

/// 既存設定と重複しないトンネル ID の入力を受け取る
fn prompt_available_tunnel_id(config: &EffectiveConfig) -> Result<String, CliError> {
    loop {
        let id = prompt_required_text("Tunnel id:")?;

        if let Some(conflict) = find_tunnel_id_conflict(config, &id) {
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Tunnel id is already used: {} ({}: {})",
                        conflict.tunnel.id,
                        conflict.source.kind,
                        conflict.source.path.display()
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
    tunnel_id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .find(|resolved| resolved.tunnel.id == tunnel_id)
}

/// 既存設定と重複しない local_port の入力を受け取る
fn prompt_available_local_port(config: &EffectiveConfig, tunnel_id: &str) -> Result<u16, CliError> {
    loop {
        let local_port = prompt_port("Local port:", None)?;

        if let Some(conflict) = find_local_port_conflict(config, tunnel_id, local_port) {
            eprintln!(
                "{}",
                red(
                    &format!(
                        "Local port is already used by tunnel: {} ({}:{})",
                        conflict.tunnel.id,
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
    tunnel_id: &str,
    local_port: u16,
) -> Option<&'a ResolvedTunnelConfig> {
    config.tunnels.iter().find(|resolved| {
        resolved.tunnel.id != tunnel_id && resolved.tunnel.local_port == local_port
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
    let choices = sorted_tunnels_by_id(&config.tunnels)
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
fn prompt_tunnels_to_stop(statuses: &[TunnelRuntimeStatus]) -> Result<Vec<String>, InquireError> {
    let sorted_statuses = sorted_statuses_by_id(statuses);
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
    let tunnels_by_id = tunnel_index_by_id(config);

    ids.iter()
        .map(|id| tunnels_by_id.get(id.as_str()).copied().ok_or(()))
        .collect()
}

/// 指定 ID のトンネル設定を取得する
fn find_tunnel_by_id<'a>(
    config: &'a EffectiveConfig,
    id: &str,
) -> Option<&'a ResolvedTunnelConfig> {
    config.tunnels.iter().find(|tunnel| tunnel.tunnel.id == id)
}

/// 統合済みトンネル設定を ID から参照する索引を生成する
fn tunnel_index_by_id(config: &EffectiveConfig) -> HashMap<&str, &ResolvedTunnelConfig> {
    config
        .tunnels
        .iter()
        .map(|resolved| (resolved.tunnel.id.as_str(), resolved))
        .collect()
}

/// トンネル状態を ID から参照する索引を生成する
fn status_index_by_id(statuses: &[TunnelRuntimeStatus]) -> HashMap<&str, &TunnelRuntimeStatus> {
    statuses
        .iter()
        .map(|status| (status.state.id.as_str(), status))
        .collect()
}

/// ID 昇順のトンネル状態一覧を生成する
fn sorted_statuses_by_id(statuses: &[TunnelRuntimeStatus]) -> Vec<&TunnelRuntimeStatus> {
    let mut statuses = statuses.iter().collect::<Vec<_>>();
    statuses.sort_by(|left, right| left.state.id.cmp(&right.state.id));

    statuses
}

/// stale なトンネル ID を取得する
fn stale_tunnel_ids(statuses: &[TunnelRuntimeStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|status| status.process_state == ProcessState::Stale)
        .map(|status| status.state.id.clone())
        .collect()
}

/// watch 対象の stale なトンネル ID を取得する
fn watched_stale_tunnel_ids(statuses: &[TunnelRuntimeStatus], ids: &[String]) -> Vec<String> {
    let requested_ids = id_set(ids);

    statuses
        .iter()
        .filter(|status| status.process_state == ProcessState::Stale)
        .filter(|status| {
            requested_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(status.state.id.as_str()))
        })
        .map(|status| status.state.id.clone())
        .collect()
}

/// watch で検出した stale なトンネルを再起動する
fn recover_watch_stale_tunnels(
    config: &EffectiveConfig,
    state_path: &Path,
    ids: &[String],
) -> Result<(), CliError> {
    for id in ids {
        let Some(tunnel) = find_tunnel_by_id(config, id) else {
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
                eprintln!("{}", red(&error.to_string(), OutputStream::Stderr));
            }
        }
    }

    Ok(())
}

/// 未知の ID を表示する
fn print_unknown_ids(config: &EffectiveConfig, ids: &[String]) {
    let known_ids = config
        .tunnels
        .iter()
        .map(|tunnel| tunnel.tunnel.id.as_str())
        .collect::<HashSet<_>>();

    for id in ids {
        if !known_ids.contains(id.as_str()) {
            eprintln!(
                "{}",
                red(&format!("Unknown tunnel id: {id}"), OutputStream::Stderr)
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
    println!("State file: {}", state_path.display());

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

    println!("Would start tunnel: {}", tunnel.id);
    println!("  Local: {local}");
    println!("  Remote: {remote}");
    println!("  SSH: {ssh}");
    println!("  Command: {command}");
}

/// stop の dry-run 結果を表示する
fn print_stop_dry_run(
    statuses: &[TunnelRuntimeStatus],
    ids: &[String],
    state_path: &Path,
) -> ExitCode {
    println!("Dry run: no process will be stopped and state file will not be modified.");
    println!("State file: {}", state_path.display());

    let statuses_by_id = status_index_by_id(statuses);
    let mut failed = false;

    for id in ids {
        let Some(status) = statuses_by_id.get(id.as_str()).copied() else {
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

        print_stop_dry_run_tunnel(status);
    }

    exit_code_from_failure(failed)
}

/// ID 一覧の一致判定用集合を生成する
fn id_set(ids: &[String]) -> Option<HashSet<&str>> {
    if ids.is_empty() {
        None
    } else {
        Some(ids.iter().map(String::as_str).collect())
    }
}

/// stop dry-run のトンネル単位の結果を表示する
fn print_stop_dry_run_tunnel(status: &TunnelRuntimeStatus) {
    match status.process_state {
        ProcessState::Running => {
            println!(
                "Would stop tunnel: {} (pid: {})",
                status.state.id, status.state.pid
            );
            println!("Would remove state entry: {}", status.state.id);
        }
        ProcessState::Stale => {
            println!(
                "Would remove stale tunnel state: {} (pid: {})",
                status.state.id, status.state.pid
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

/// 状態一覧の見出しを表示する
fn print_status_header(widths: StatusColumnWidths) {
    println!(
        "{} {} {} {} {} STARTED",
        pad_display_width("ID", widths.id),
        pad_display_width("LOCAL", widths.local),
        pad_display_width("REMOTE", widths.remote),
        pad_display_width("PID", widths.pid),
        pad_display_width("STATE", widths.state)
    );
}

/// 状態一覧の 1 行を表示する
fn print_status_row(row: &StatusRow<'_>, widths: StatusColumnWidths) {
    println!(
        "{} {} {} {} {} {}",
        pad_display_width_with_visible_width(row.id, row.id_width, widths.id),
        pad_display_width_with_visible_width(&row.local, row.local_width, widths.local),
        pad_display_width_with_visible_width(&row.remote, row.remote_width, widths.remote),
        pad_display_width_with_visible_width(&row.pid, row.pid_width, widths.pid),
        pad_display_width_with_visible_width(
            &row.process_state,
            row.process_state_width,
            widths.state,
        ),
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
            id: state.id.as_str(),
            id_width: display_width(&state.id),
            local_width: display_width(&local),
            local,
            remote_width: display_width(&remote),
            remote,
            pid_width: display_width(&pid),
            pid,
            process_state_width,
            process_state,
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

#[cfg(test)]
mod tests {
    use fwd_deck_core::{ConfigSource, TunnelState, config::LoadedConfigFile};

    use super::*;

    /// 既存トンネルと同じ ID が検出されることを検証する
    #[test]
    fn find_tunnel_id_conflict_detects_existing_tunnel() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_tunnel_id_conflict(&config, "db");

        assert_eq!(
            conflict.map(|resolved| resolved.tunnel.id.as_str()),
            Some("db")
        );
    }

    /// 未使用の ID は重複扱いしないことを検証する
    #[test]
    fn find_tunnel_id_conflict_returns_none_for_new_tunnel() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_tunnel_id_conflict(&config, "cache");

        assert!(conflict.is_none());
    }

    /// 同じ local_port を使う別 ID のトンネルが検出されることを検証する
    #[test]
    fn find_local_port_conflict_detects_other_tunnel() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_local_port_conflict(&config, "cache", 15432);

        assert_eq!(
            conflict.map(|resolved| resolved.tunnel.id.as_str()),
            Some("db")
        );
    }

    /// 同一 ID の既存トンネルは上書き候補として重複扱いしないことを検証する
    #[test]
    fn find_local_port_conflict_ignores_same_tunnel_id() {
        let config = effective_config_with_tunnels(vec![tunnel("db", 15432)]);

        let conflict = find_local_port_conflict(&config, "db", 15432);

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

        let ids = watched_stale_tunnel_ids(&statuses, &[]);

        assert_eq!(ids, vec!["db".to_owned(), "search".to_owned()]);
    }

    /// watch 対象指定時に指定 ID の stale なトンネルだけが対象になることを検証する
    #[test]
    fn watched_stale_tunnel_ids_filters_by_requested_ids() {
        let statuses = vec![
            runtime_status("db", ProcessState::Stale),
            runtime_status("cache", ProcessState::Stale),
            runtime_status("search", ProcessState::Running),
        ];

        let ids = watched_stale_tunnel_ids(&statuses, &["cache".to_owned(), "search".to_owned()]);

        assert_eq!(ids, vec!["cache".to_owned()]);
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
        assert_eq!(tunnels[0].tunnel.id, "dev-db");
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

        let tunnel = find_tunnel_by_id(&config, "dev-db");

        assert_eq!(
            tunnel.map(|resolved| resolved.tunnel.id.as_str()),
            Some("dev-db")
        );
    }

    /// show 対象の ID が部分一致では取得されないことを検証する
    #[test]
    fn find_tunnel_by_id_does_not_return_partial_match() {
        let config = effective_config_with_tunnels(vec![tunnel("dev-db", 15432)]);

        let tunnel = find_tunnel_by_id(&config, "dev");

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
            .map(|resolved| resolved.tunnel.id.as_str())
            .collect()
    }

    /// テスト用の状態 ID 一覧を取得する
    fn status_ids<'a>(statuses: &[&'a TunnelRuntimeStatus]) -> Vec<&'a str> {
        statuses
            .iter()
            .map(|status| status.state.id.as_str())
            .collect()
    }

    /// テスト用のトンネル実行状態を生成する
    fn runtime_status(id: &str, process_state: ProcessState) -> TunnelRuntimeStatus {
        TunnelRuntimeStatus {
            state: TunnelState {
                id: id.to_owned(),
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
