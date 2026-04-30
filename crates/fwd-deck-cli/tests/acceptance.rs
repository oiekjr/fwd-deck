use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{self, Command, Output},
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

/// list が ID 昇順でトンネルを表示することを検証する
#[test]
fn list_sorts_tunnels_by_id() {
    let workspace = TestWorkspace::new();
    workspace.write_config(unsorted_config());

    let output = workspace.run(["list"]);

    assert!(output.status.success());
    output.assert_stdout_order(["dev-db", "prod-cache"]);
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

/// list --json が設定済みトンネルを JSON で表示することを検証する
#[test]
fn list_json_outputs_configured_tunnels() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["--json", "list", "--query", "cache"]);

    assert!(output.status.success());
    let json = output.stdout_json();
    assert_eq!(json["hasConfig"], true);
    assert_eq!(json["tunnels"][0]["id"], "prod-cache");
    assert_eq!(json["tunnels"][0]["localPort"], 16379);
}

/// list --wide が REMOTE の省略有無を切り替えることを検証する
#[test]
fn list_wide_displays_full_remote_host() {
    let workspace = TestWorkspace::new();
    workspace.write_config(long_remote_config());

    let output = workspace.run(["list"]);

    assert!(output.status.success());
    output.assert_stdout_contains("...:5432");
    output.assert_stdout_not_contains(
        "japandb-as.cluster-clpmwhbh0sfa.ap-northeast-1.rds.amazonaws.com:5432",
    );

    let output = workspace.run(["list", "--wide"]);

    assert!(output.status.success());
    output.assert_stdout_contains(
        "japandb-as.cluster-clpmwhbh0sfa.ap-northeast-1.rds.amazonaws.com:5432",
    );
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

/// validate --json が検証結果を JSON で表示することを検証する
#[test]
fn validate_json_outputs_validation_report() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run(["--json", "validate"]);

    assert!(output.status.success());
    let json = output.stdout_json();
    assert_eq!(json["hasConfig"], true);
    assert_eq!(json["isValid"], true);
    assert!(
        json["errors"]
            .as_array()
            .expect("errors is array")
            .is_empty()
    );
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

/// start --dry-run --json が開始予定を JSON で表示することを検証する
#[test]
fn start_dry_run_json_outputs_plan_without_writing_state() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run([
        "--state",
        workspace.state_path_str(),
        "--json",
        "start",
        "dev-db",
        "--dry-run",
    ]);

    assert!(output.status.success());
    let json = output.stdout_json();
    assert_eq!(json["dryRun"], true);
    assert_eq!(json["tunnels"][0]["tunnel"]["id"], "dev-db");
    assert!(
        json["tunnels"][0]["command"]
            .as_str()
            .expect("command is string")
            .starts_with("ssh ")
    );
    assert!(!workspace.state_path().exists());
}

/// start --all --dry-run が ID 昇順で開始予定を表示することを検証する
#[test]
fn start_all_dry_run_sorts_tunnels_by_id() {
    let workspace = TestWorkspace::new();
    workspace.write_config(unsorted_config());

    let output = workspace.run([
        "--state",
        workspace.state_path_str(),
        "start",
        "--all",
        "--dry-run",
    ]);

    assert!(output.status.success());
    output.assert_stdout_order([
        "Would start tunnel: dev-db",
        "Would start tunnel: prod-cache",
    ]);
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

/// --help が日本語の概要、主要コマンド、代表例を表示することを検証する
#[test]
fn help_displays_japanese_overview_commands_and_examples() {
    let workspace = TestWorkspace::new();

    let output = workspace.run(["--help"]);

    assert!(output.status.success());
    output.assert_stdout_contains("設定ファイルに定義したポートフォワーディングを操作する");
    output.assert_stdout_contains("設定済みトンネルを一覧表示する");
    output.assert_stdout_contains("設定ファイルを検証する");
    output.assert_stdout_contains("fwd-deck start dev-db --dry-run");
}

/// start --help が対話選択、排他制約、dry-run の説明を表示することを検証する
#[test]
fn start_help_displays_selection_constraints_and_dry_run_note() {
    let workspace = TestWorkspace::new();

    let output = workspace.run(["start", "--help"]);

    assert!(output.status.success());
    output.assert_stdout_contains("ID を省略すると対話選択を表示します。");
    output.assert_stdout_contains("--all、ID、--tag は同時に指定できません。");
    output.assert_stdout_contains("--dry-run は SSH を起動せず、状態ファイルも更新しません。");
}

/// list --help が tag、query、wide の説明を表示することを検証する
#[test]
fn list_help_displays_filter_and_wide_notes() {
    let workspace = TestWorkspace::new();

    let output = workspace.run(["list", "--help"]);

    assert!(output.status.success());
    output.assert_stdout_contains("--tag は複数指定でき");
    output.assert_stdout_contains("--query は ID と description");
    output.assert_stdout_contains("--wide は REMOTE の host 部分を省略せずに表示します。");
}

/// config add --help が scope 省略時の対話選択を表示することを検証する
#[test]
fn config_add_help_displays_scope_selection_note() {
    let workspace = TestWorkspace::new();

    let output = workspace.run(["config", "add", "--help"]);

    assert!(output.status.success());
    output.assert_stdout_contains(
        "--scope を省略すると、編集する local または global 設定を対話選択します。",
    );
    output.assert_stdout_contains(
        "local は ./fwd-deck.toml、global は ~/.config/fwd-deck/config.toml",
    );
}

/// doctor が設定ファイルなしを失敗として診断することを検証する
#[test]
fn doctor_fails_when_configuration_is_missing() {
    let workspace = TestWorkspace::new();

    let output = workspace.run(["--state", workspace.state_path_str(), "doctor"]);

    assert!(!output.status.success());
    output.assert_stdout_contains("Doctor report");
    output.assert_stdout_contains("[ERROR] Configuration files");
}

/// 人間向け出力ではホームディレクトリ配下のパスをチルダで表示することを検証する
#[test]
fn human_output_shortens_home_paths() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run_with_home(["show", "dev-db"], workspace.path());

    assert!(output.status.success());
    output.assert_stdout_contains("Source: local (~/fwd-deck.toml)");

    let output = workspace.run_with_home(
        [
            "--state",
            workspace.state_path_str(),
            "start",
            "dev-db",
            "--dry-run",
        ],
        workspace.path(),
    );

    assert!(output.status.success());
    output.assert_stdout_contains("State file: ~/state.toml");
}

/// JSON出力ではホームディレクトリ配下のパスを絶対パスのまま保持することを検証する
#[test]
fn json_output_keeps_absolute_home_paths() {
    let workspace = TestWorkspace::new();
    workspace.write_config(valid_config());

    let output = workspace.run_with_home(
        [
            "--state",
            workspace.state_path_str(),
            "--json",
            "start",
            "dev-db",
            "--dry-run",
        ],
        workspace.path(),
    );

    assert!(output.status.success());
    let json = output.stdout_json();
    assert_eq!(json["stateFile"], workspace.state_path_str());
    assert_eq!(
        json["tunnels"][0]["tunnel"]["sourcePath"],
        workspace.config_path_str()
    );
}

/// status が記録PIDによるLISTEN状態をRUNNINGとして表示することを検証する
#[test]
fn status_displays_running_when_tracked_pid_listens_on_local_port() {
    let workspace = TestWorkspace::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let local_port = listener.local_addr().expect("read listener address").port();
    workspace.write_state(&state_file(process::id(), local_port));

    let output = workspace.run(["--state", workspace.state_path_str(), "status"]);

    assert!(output.status.success());
    output.assert_stdout_contains("dev-db");
    output.assert_stdout_contains("RUNNING");
}

/// status が記録PIDだけ存在する状態をSTALEとして表示することを検証する
#[test]
fn status_displays_stale_when_tracked_pid_does_not_listen_on_local_port() {
    let workspace = TestWorkspace::new();
    let local_port = unused_local_port();
    workspace.write_state(&state_file(process::id(), local_port));

    let output = workspace.run(["--state", workspace.state_path_str(), "status"]);

    assert!(output.status.success());
    output.assert_stdout_contains("dev-db");
    output.assert_stdout_contains("STALE");
    output.assert_stdout_not_contains("RUNNING");
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

    /// テスト用状態ファイルを書き込む
    fn write_state(&self, content: &str) {
        fs::write(&self.state_path, content).expect("write test state");
    }

    /// fwd-deck を一時ワークスペース上で実行する
    fn run<const N: usize>(&self, args: [&str; N]) -> CommandOutput {
        let output = self.command(args).output().expect("run fwd-deck");

        CommandOutput::from_output(output)
    }

    /// ホームディレクトリを差し替えて fwd-deck を一時ワークスペース上で実行する
    fn run_with_home<const N: usize>(&self, args: [&str; N], home: &Path) -> CommandOutput {
        let output = self
            .command(args)
            .env("HOME", home)
            .output()
            .expect("run fwd-deck");

        CommandOutput::from_output(output)
    }

    /// fwd-deck 実行コマンドを初期化する
    fn command<const N: usize>(&self, args: [&str; N]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fwd-deck"));
        command
            .current_dir(self.temp_dir.path())
            .arg("--config")
            .arg(&self.config_path)
            .arg("--no-global")
            .args(args);

        command
    }

    /// 一時ワークスペースのパスを取得する
    fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// 設定ファイルのパスを CLI 引数用文字列として取得する
    fn config_path_str(&self) -> &str {
        self.config_path
            .to_str()
            .expect("configuration path must be valid UTF-8")
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

    /// stdout 内で期待文字列が指定順に現れることを検証する
    fn assert_stdout_order<const N: usize>(&self, expected_values: [&str; N]) {
        let mut offset = 0;

        for expected in expected_values {
            let remaining = &self.stdout[offset..];
            let Some(index) = remaining.find(expected) else {
                panic!(
                    "stdout did not contain {expected:?} after offset {offset}\nstdout:\n{}\nstderr:\n{}",
                    self.stdout, self.stderr
                );
            };

            offset += index + expected.len();
        }
    }

    /// stdout を JSON として解析する
    fn stdout_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|error| {
            panic!(
                "stdout was not valid JSON: {error}\nstdout:\n{}\nstderr:\n{}",
                self.stdout, self.stderr
            )
        })
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

/// ID 順とは逆に記述されたテスト用設定を生成する
fn unsorted_config() -> &'static str {
    r#"
[[tunnels]]
id = "prod-cache"
local_host = "127.0.0.1"
local_port = 16379
remote_host = "127.0.0.1"
remote_port = 6379
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"

[[tunnels]]
id = "dev-db"
local_host = "127.0.0.1"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
"#
}

/// 長い remote host を持つテスト用設定を生成する
fn long_remote_config() -> &'static str {
    r#"
[[tunnels]]
id = "prod-db"
local_host = "127.0.0.1"
local_port = 15432
remote_host = "japandb-as.cluster-clpmwhbh0sfa.ap-northeast-1.rds.amazonaws.com"
remote_port = 5432
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

/// テスト用状態ファイルを生成する
fn state_file(pid: u32, local_port: u16) -> String {
    format!(
        r#"
[[tunnels]]
id = "dev-db"
pid = {pid}
local_host = "127.0.0.1"
local_port = {local_port}
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
ssh_port = 22
source_kind = "local"
source_path = "fwd-deck.toml"
started_at_unix_seconds = 1700000000
"#
    )
}

/// 未使用のローカルポート番号を取得する
fn unused_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");

    listener.local_addr().expect("read listener address").port()
}
