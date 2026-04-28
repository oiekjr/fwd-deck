use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tempfile::TempDir;

/// list が設定済みトンネルを表示することを検証する
#[test]
fn list_displays_configured_tunnels() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["list"]);

    assert!(output.status.success());
    output.assert_stdout_contains("dev-db");
    output.assert_stdout_contains("prod-cache");
    output.assert_stdout_contains("dev,project-a");
}

/// list --tag が指定タグを持つトンネルだけを表示することを検証する
#[test]
fn list_tag_filters_tunnels_by_tag() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["list", "--tag", "dev"]);

    assert!(output.status.success());
    output.assert_stdout_contains("dev-db");
    output.assert_stdout_not_contains("prod-cache");
}

/// list --query が description の部分一致でトンネルを表示することを検証する
#[test]
fn list_query_matches_description() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["list", "--query", "cache"]);

    assert!(output.status.success());
    output.assert_stdout_contains("prod-cache");
    output.assert_stdout_not_contains("dev-db");
}

/// show が description を含むトンネル詳細を表示することを検証する
#[test]
fn show_displays_tunnel_details() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["show", "dev-db"]);

    assert!(output.status.success());
    output.assert_stdout_contains("ID: dev-db");
    output.assert_stdout_contains("Description: Development database");
    output.assert_stdout_contains("Tags: dev,project-a");
    output.assert_stdout_contains("Connect timeout: 10s");
}

/// show が存在しない ID を失敗として扱うことを検証する
#[test]
fn show_fails_for_unknown_tunnel_id() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["show", "missing"]);

    assert!(!output.status.success());
    output.assert_stderr_contains("No tunnel matched ID: missing.");
}

/// validate が有効な設定を成功として扱うことを検証する
#[test]
fn validate_succeeds_for_valid_config() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["validate"]);

    assert!(output.status.success());
    output.assert_stdout_contains("Configuration is valid.");
}

/// validate が重複 local_port を失敗として扱うことを検証する
#[test]
fn validate_fails_for_duplicate_local_port() {
    let workspace = TestWorkspace::new();
    workspace.write_config(duplicate_local_port_config());

    let output = workspace.run(["validate"]);

    assert!(!output.status.success());
    output.assert_stderr_contains("Configuration has errors.");
    output.assert_stderr_contains("local_port 15432 duplicates dev-db");
}

/// start --dry-run が SSH 起動と状態ファイル更新を行わないことを検証する
#[test]
fn start_dry_run_prints_plan_without_writing_state() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run([
        "--state",
        workspace.state_path_str(),
        "start",
        "--all",
        "--dry-run",
    ]);

    assert!(output.status.success());
    output.assert_stdout_contains("Dry run: no ssh process will be started");
    output.assert_stdout_contains("Would start tunnel: dev-db");
    output.assert_stdout_contains("Would start tunnel: prod-cache");
    assert!(!workspace.state_path().exists());
}

/// start が ID と --tag の同時指定を失敗として扱うことを検証する
#[test]
fn start_fails_when_id_and_tag_are_combined() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["start", "dev-db", "--tag", "dev", "--dry-run"]);

    assert!(!output.status.success());
    output.assert_stderr_contains("Cannot combine tunnel IDs with --tag.");
}

struct TestWorkspace {
    temp_dir: TempDir,
    config_path: PathBuf,
    state_path: PathBuf,
}

impl TestWorkspace {
    /// テスト用ワークスペースを初期化する
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("create temporary directory");
        let config_path = temp_dir.path().join("fwd-deck.toml");
        let state_path = temp_dir.path().join("state.toml");

        Self {
            temp_dir,
            config_path,
            state_path,
        }
    }

    /// テスト用設定ファイルを書き込む
    fn write_config(&self, content: &str) {
        fs::write(&self.config_path, content).expect("write test configuration");
    }

    /// fwd-deck を一時ワークスペース上で実行する
    fn run<const N: usize>(&self, args: [&str; N]) -> CommandOutput {
        let output = Command::new(env!("CARGO_BIN_EXE_fwd-deck"))
            .current_dir(self.temp_dir.path())
            .arg("--config")
            .arg(&self.config_path)
            .arg("--no-global")
            .args(args)
            .output()
            .expect("run fwd-deck");

        CommandOutput::from_output(output)
    }

    /// 状態ファイルのパスを取得する
    fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// 状態ファイルのパスを CLI 引数用文字列として取得する
    fn state_path_str(&self) -> &str {
        self.state_path
            .to_str()
            .expect("state path must be valid UTF-8")
    }
}

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

impl CommandOutput {
    /// process output を検証しやすい形式へ変換する
    fn from_output(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: String::from_utf8(output.stdout).expect("stdout must be valid UTF-8"),
            stderr: String::from_utf8(output.stderr).expect("stderr must be valid UTF-8"),
        }
    }

    /// stdout に期待文字列が含まれることを検証する
    fn assert_stdout_contains(&self, expected: &str) {
        assert!(
            self.stdout.contains(expected),
            "stdout did not contain {expected:?}\nstdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
    }

    /// stdout に期待文字列が含まれないことを検証する
    fn assert_stdout_not_contains(&self, unexpected: &str) {
        assert!(
            !self.stdout.contains(unexpected),
            "stdout contained {unexpected:?}\nstdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
    }

    /// stderr に期待文字列が含まれることを検証する
    fn assert_stderr_contains(&self, expected: &str) {
        assert!(
            self.stderr.contains(expected),
            "stderr did not contain {expected:?}\nstdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
    }
}

/// 有効なテスト用設定を生成する
fn valid_config() -> &'static str {
    r#"
[timeouts]
connect_timeout_seconds = 15
server_alive_interval_seconds = 30
server_alive_count_max = 3
start_grace_milliseconds = 300

[[tunnels]]
id = "dev-db"
description = "Development database"
tags = ["dev", "project-a"]
local_host = "127.0.0.1"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
ssh_port = 22
identity_file = "~/.ssh/id_ed25519"

[tunnels.timeouts]
connect_timeout_seconds = 10

[[tunnels]]
id = "prod-cache"
description = "Production cache"
tags = ["prod", "project-a"]
local_host = "127.0.0.1"
local_port = 16379
remote_host = "127.0.0.1"
remote_port = 6379
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
"#
}

/// local_port が重複するテスト用設定を生成する
fn duplicate_local_port_config() -> &'static str {
    r#"
[[tunnels]]
id = "dev-db"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"

[[tunnels]]
id = "dev-cache"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 6379
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
"#
}
