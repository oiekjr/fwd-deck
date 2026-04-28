use std::{
    env,
    io::{self, IsTerminal},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use fwd_deck_core::{
    ConfigPaths, EffectiveConfig, ResolvedTunnelConfig, ValidationReport,
    default_global_config_path, default_local_config_path, load_effective_config, validate_config,
};
use thiserror::Error;

/// fwd-deck の CLI 引数を表現する
#[derive(Debug, Parser)]
#[command(
    name = "fwd-deck",
    version,
    about = "Operate port forwarding entries defined in configuration files"
)]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[arg(long, global = true, value_name = "PATH")]
    global_config: Option<PathBuf>,

    #[arg(long, global = true)]
    no_global: bool,

    #[command(subcommand)]
    command: Command,
}

/// fwd-deck が提供するサブコマンドを表現する
#[derive(Debug, Subcommand)]
enum Command {
    List,
    Validate,
}

/// CLI 実行時の失敗理由を表現する
#[derive(Debug, Error)]
enum CliError {
    #[error("Failed to get the current directory: {0}")]
    CurrentDir(std::io::Error),
    #[error(transparent)]
    Config(#[from] fwd_deck_core::ConfigLoadError),
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
    let paths = resolve_config_paths(&cli)?;
    let config = load_effective_config(&paths)?;

    match cli.command {
        Command::List => {
            print_list(&config);
            Ok(ExitCode::SUCCESS)
        }
        Command::Validate => Ok(print_validation(&config)),
    }
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
