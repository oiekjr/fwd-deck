use std::{
    env, io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{
    ResolvedTunnelConfig, TunnelState,
    state::{StateFileError, read_state_file, write_state_file},
};

const SSH_CONNECT_TIMEOUT_SECONDS: &str = "15";
const SSH_SERVER_ALIVE_INTERVAL_SECONDS: &str = "30";
const SSH_SERVER_ALIVE_COUNT_MAX: &str = "3";
const SSH_START_GRACE_PERIOD: Duration = Duration::from_millis(300);

/// 起動中プロセスの状態を表現する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Stale,
}

/// 起動処理で開始されたトンネルを表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedTunnel {
    pub state: TunnelState,
}

/// 停止処理で扱ったトンネルを表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoppedTunnel {
    pub state: TunnelState,
    pub previous_state: ProcessState,
}

/// 状態確認時のトンネル情報を表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRuntimeStatus {
    pub state: TunnelState,
    pub process_state: ProcessState,
}

/// トンネル実行時の失敗理由を表現する
#[derive(Debug, Error)]
pub enum TunnelRuntimeError {
    #[error(transparent)]
    State(#[from] StateFileError),
    #[error("Tunnel is already running: {id} (pid: {pid})")]
    AlreadyRunning { id: String, pid: u32 },
    #[error("Tunnel is not tracked: {id}")]
    NotTracked { id: String },
    #[error("Local endpoint is not available: {id} ({local_host}:{local_port}): {source}")]
    LocalEndpointUnavailable {
        id: String,
        local_host: String,
        local_port: u16,
        source: io::Error,
    },
    #[error("Failed to start ssh for tunnel: {id}: {source}")]
    Spawn { id: String, source: io::Error },
    #[error("Failed to inspect ssh startup for tunnel: {id}: {source}")]
    StartupCheck { id: String, source: io::Error },
    #[error("ssh exited before the tunnel was ready: {id}: {status}")]
    EarlyExit {
        id: String,
        status: std::process::ExitStatus,
    },
    #[error("Failed to stop tunnel: {id} (pid: {pid}): {source}")]
    Stop {
        id: String,
        pid: u32,
        source: io::Error,
    },
}

/// トンネルを開始して状態ファイルへ保存する
pub fn start_tunnel(
    resolved: &ResolvedTunnelConfig,
    state_path: &Path,
) -> Result<StartedTunnel, TunnelRuntimeError> {
    let mut state_file = read_state_file(state_path)?;

    if let Some(existing) = state_file.get(&resolved.tunnel.id)
        && process_is_running(existing.pid)
    {
        return Err(TunnelRuntimeError::AlreadyRunning {
            id: existing.id.clone(),
            pid: existing.pid,
        });
    }

    ensure_local_endpoint_available(resolved)?;

    let mut child = spawn_ssh_tunnel(resolved)?;
    thread::sleep(SSH_START_GRACE_PERIOD);

    if let Some(status) = child
        .try_wait()
        .map_err(|source| TunnelRuntimeError::StartupCheck {
            id: resolved.tunnel.id.clone(),
            source,
        })?
    {
        return Err(TunnelRuntimeError::EarlyExit {
            id: resolved.tunnel.id.clone(),
            status,
        });
    }

    let started_at_unix_seconds = current_unix_seconds();
    let tunnel_state =
        TunnelState::from_resolved_tunnel(resolved, child.id(), started_at_unix_seconds);
    state_file.upsert(tunnel_state.clone());
    write_state_file(state_path, &state_file)?;

    Ok(StartedTunnel {
        state: tunnel_state,
    })
}

/// トンネル状態の一覧を取得する
pub fn tunnel_statuses(state_path: &Path) -> Result<Vec<TunnelRuntimeStatus>, TunnelRuntimeError> {
    let state_file = read_state_file(state_path)?;

    Ok(state_file
        .tunnels
        .into_iter()
        .map(|state| {
            let process_state = if process_is_running(state.pid) {
                ProcessState::Running
            } else {
                ProcessState::Stale
            };

            TunnelRuntimeStatus {
                state,
                process_state,
            }
        })
        .collect())
}

/// トンネルを停止して状態ファイルから削除する
pub fn stop_tunnel(id: &str, state_path: &Path) -> Result<StoppedTunnel, TunnelRuntimeError> {
    let mut state_file = read_state_file(state_path)?;
    let Some(state) = state_file.remove(id) else {
        return Err(TunnelRuntimeError::NotTracked { id: id.to_owned() });
    };
    let previous_state = if process_is_running(state.pid) {
        stop_process(&state)?;
        ProcessState::Running
    } else {
        ProcessState::Stale
    };

    write_state_file(state_path, &state_file)?;

    Ok(StoppedTunnel {
        state,
        previous_state,
    })
}

/// SSH 起動引数を構築する
fn build_ssh_args(resolved: &ResolvedTunnelConfig) -> Vec<String> {
    let tunnel = &resolved.tunnel;
    let local_forward = format!(
        "{}:{}:{}:{}",
        tunnel.effective_local_host(),
        tunnel.local_port,
        tunnel.remote_host,
        tunnel.remote_port
    );
    let mut args = vec![
        "-N".to_owned(),
        "-L".to_owned(),
        local_forward,
        "-o".to_owned(),
        "ExitOnForwardFailure=yes".to_owned(),
        "-o".to_owned(),
        format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECONDS}"),
        "-o".to_owned(),
        format!("ServerAliveInterval={SSH_SERVER_ALIVE_INTERVAL_SECONDS}"),
        "-o".to_owned(),
        format!("ServerAliveCountMax={SSH_SERVER_ALIVE_COUNT_MAX}"),
    ];

    if let Some(port) = tunnel.ssh_port {
        args.push("-p".to_owned());
        args.push(port.to_string());
    }

    if let Some(identity_file) = &tunnel.identity_file {
        args.push("-i".to_owned());
        args.push(expand_home_path(identity_file));
    }

    args.push(format!("{}@{}", tunnel.ssh_user, tunnel.ssh_host));

    args
}

/// SSH 子プロセスを起動する
fn spawn_ssh_tunnel(
    resolved: &ResolvedTunnelConfig,
) -> Result<std::process::Child, TunnelRuntimeError> {
    Command::new("ssh")
        .args(build_ssh_args(resolved))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| TunnelRuntimeError::Spawn {
            id: resolved.tunnel.id.clone(),
            source,
        })
}

/// ローカル側エンドポイントを利用できるかを検証する
fn ensure_local_endpoint_available(
    resolved: &ResolvedTunnelConfig,
) -> Result<(), TunnelRuntimeError> {
    let tunnel = &resolved.tunnel;

    TcpListener::bind((tunnel.effective_local_host(), tunnel.local_port))
        .map(drop)
        .map_err(|source| TunnelRuntimeError::LocalEndpointUnavailable {
            id: tunnel.id.clone(),
            local_host: tunnel.effective_local_host().to_owned(),
            local_port: tunnel.local_port,
            source,
        })
}

/// プロセスが存在するかを判定する
fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// プロセスへ終了シグナルを送信する
fn stop_process(state: &TunnelState) -> Result<(), TunnelRuntimeError> {
    Command::new("kill")
        .arg("-TERM")
        .arg(state.pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| TunnelRuntimeError::Stop {
            id: state.id.clone(),
            pid: state.pid,
            source,
        })
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(TunnelRuntimeError::Stop {
                    id: state.id.clone(),
                    pid: state.pid,
                    source: io::Error::other(format!("kill exited with {status}")),
                })
            }
        })
}

/// 現在時刻を UNIX 秒で取得する
fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// `~/` で始まるパスを HOME 配下の絶対パスへ展開する
fn expand_home_path(path: &str) -> String {
    let Some(rest) = path.strip_prefix("~/") else {
        return path.to_owned();
    };

    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(rest).display().to_string())
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{ConfigSource, ConfigSourceKind, TunnelConfig};

    use super::*;

    /// SSH 起動引数が設定値から生成されることを検証する
    #[test]
    fn build_ssh_args_uses_tunnel_configuration() {
        let resolved = resolved_tunnel();

        let args = build_ssh_args(&resolved);

        assert!(args.contains(&"-N".to_owned()));
        assert!(args.contains(&"-L".to_owned()));
        assert!(args.contains(&"127.0.0.1:15432:db.internal:5432".to_owned()));
        assert!(args.contains(&"ConnectTimeout=15".to_owned()));
        assert!(args.contains(&"user@bastion.example.com".to_owned()));
    }

    /// `~/` 形式の identity_file が HOME 配下へ展開されることを検証する
    #[test]
    fn build_ssh_args_expands_home_in_identity_file() {
        let resolved = resolved_tunnel();

        let args = build_ssh_args(&resolved);

        assert!(args.iter().any(|arg| arg.ends_with("/.ssh/id_ed25519")));
    }

    /// テスト用の統合済みトンネル設定を生成する
    fn resolved_tunnel() -> ResolvedTunnelConfig {
        ResolvedTunnelConfig::new(
            ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml")),
            TunnelConfig {
                id: "db".to_owned(),
                description: None,
                local_host: Some("127.0.0.1".to_owned()),
                local_port: 15432,
                remote_host: "db.internal".to_owned(),
                remote_port: 5432,
                ssh_user: "user".to_owned(),
                ssh_host: "bastion.example.com".to_owned(),
                ssh_port: None,
                identity_file: Some("~/.ssh/id_ed25519".to_owned()),
            },
        )
    }
}
