use std::process::ExitCode;

/// CLI binary の実行入口を初期化する
fn main() -> ExitCode {
    fwd_deck_cli::run_from_env()
}
