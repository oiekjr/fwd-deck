use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::{self, Display},
    io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{
    ResolvedTimeoutConfig, ResolvedTunnelConfig, TunnelState, TunnelStateFile,
    state::{
        StateFileError, normalize_runtime_source_path, read_state_file,
        runtime_id_for_resolved_tunnel, tunnel_runtime_id_from_normalized_source_path,
        write_state_file,
    },
};

const STARTUP_LISTEN_CONFIRMATION_MILLISECONDS: u64 = 1_000;
const STARTUP_PROBE_INTERVAL_MILLISECONDS: u64 = 25;

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

/// ローカルポートを使用しているプロセス一覧を表現する
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalPortProcesses(Vec<LocalPortProcess>);

impl LocalPortProcesses {
    /// プロセス一覧を初期化する
    pub fn new(processes: Vec<LocalPortProcess>) -> Self {
        Self(processes)
    }

    /// プロセス一覧が空かを判定する
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 指定PIDを含むかを判定する
    pub fn contains_pid(&self, pid: u32) -> bool {
        self.0.iter().any(|process| process.pid == pid)
    }
}

impl Display for LocalPortProcesses {
    /// エラーメッセージへ付加するプロセス情報を出力する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return Ok(());
        }

        let process_labels = self
            .0
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let noun = if self.0.len() == 1 {
            "listening process"
        } else {
            "listening processes"
        };

        write!(formatter, "; {noun}: {process_labels}")
    }
}

/// ローカルポートを使用しているプロセスを表現する
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPortProcess {
    pub command: String,
    pub pid: u32,
    pub endpoint: Option<String>,
}

impl Display for LocalPortProcess {
    /// プロセス情報を表示用文字列へ変換する
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (pid: {}", self.command, self.pid)?;

        if let Some(endpoint) = &self.endpoint {
            write!(formatter, ", endpoint: {endpoint}")?;
        }

        write!(formatter, ")")
    }
}

/// トンネル実行時の失敗理由を表現する
#[derive(Debug, Error)]
pub enum TunnelRuntimeError {
    #[error(transparent)]
    State(#[from] StateFileError),
    #[error("Tunnel is already running: {name} (pid: {pid})")]
    AlreadyRunning {
        runtime_id: String,
        name: String,
        pid: u32,
    },
    #[error("Tunnel is not tracked: {runtime_id}")]
    NotTracked { runtime_id: String },
    #[error(
        "Local endpoint is not available: {name} ({local_host}:{local_port}): {source}{processes}"
    )]
    LocalEndpointUnavailable {
        name: String,
        local_host: String,
        local_port: u16,
        source: io::Error,
        processes: LocalPortProcesses,
    },
    #[error("Failed to start ssh for tunnel: {name}: {source}")]
    Spawn { name: String, source: io::Error },
    #[error("Failed to inspect ssh startup for tunnel: {name}: {source}")]
    StartupCheck { name: String, source: io::Error },
    #[error("Tunnel start worker panicked: {name}")]
    StartWorkerPanic { name: String },
    #[error("ssh exited before the tunnel was ready: {name}: {status}")]
    EarlyExit {
        name: String,
        status: std::process::ExitStatus,
    },
    #[error("Failed to stop tunnel: {name} (pid: {pid}): {source}")]
    Stop {
        name: String,
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
    let runtime_id = runtime_id_for_resolved_tunnel(resolved);

    if let Some(existing) = state_file.get(&runtime_id)
        && tunnel_is_running(existing)
    {
        return Err(TunnelRuntimeError::AlreadyRunning {
            runtime_id: existing.runtime_id.clone(),
            name: existing.name.clone(),
            pid: existing.pid,
        });
    }

    let started = start_tunnel_without_state_write(resolved)?;
    state_file.upsert(started.state.clone());
    write_state_file(state_path, &state_file)?;

    Ok(started)
}

/// 複数トンネルを状態ファイル単位で開始する
pub fn start_tunnels(
    resolved_tunnels: &[ResolvedTunnelConfig],
    state_path: &Path,
    parallelism: usize,
) -> Result<Vec<Result<StartedTunnel, TunnelRuntimeError>>, TunnelRuntimeError> {
    start_tunnels_with_progress(
        resolved_tunnels,
        state_path,
        parallelism,
        |_index, _result| {},
    )
}

/// 複数トンネルを状態ファイル単位で開始し、各結果確定時に通知する
pub fn start_tunnels_with_progress<F>(
    resolved_tunnels: &[ResolvedTunnelConfig],
    state_path: &Path,
    parallelism: usize,
    mut on_result: F,
) -> Result<Vec<Result<StartedTunnel, TunnelRuntimeError>>, TunnelRuntimeError>
where
    F: FnMut(usize, &Result<StartedTunnel, TunnelRuntimeError>),
{
    let mut state_file = read_state_file(state_path)?;
    let mut results = (0..resolved_tunnels.len())
        .map(|_| None)
        .collect::<Vec<_>>();
    let mut pending_jobs = Vec::new();
    let existing_state_probe = RuntimeStatusProbe::from_states(&state_file.tunnels);
    let existing_states = index_tunnel_states_by_runtime_id(&state_file);
    let mut runtime_id_cache = RuntimeIdCache::default();

    for (index, resolved) in resolved_tunnels.iter().enumerate() {
        let runtime_id = runtime_id_cache.runtime_id_for_resolved_tunnel(resolved);

        if let Some(existing) = existing_states.get(runtime_id.as_str())
            && existing_state_probe.process_state_for(existing) == ProcessState::Running
        {
            let result = Err(TunnelRuntimeError::AlreadyRunning {
                runtime_id: existing.runtime_id.clone(),
                name: existing.name.clone(),
                pid: existing.pid,
            });
            on_result(index, &result);
            results[index] = Some(result);
            continue;
        }

        pending_jobs.push(StartTunnelJob {
            index,
            resolved: resolved.clone(),
        });
    }

    let parallelism = parallelism.max(1);
    for chunk in pending_jobs.chunks(parallelism) {
        let handles = chunk
            .iter()
            .map(|job| {
                let resolved = job.resolved.clone();
                thread::spawn(move || start_tunnel_without_state_write(&resolved))
            })
            .collect::<Vec<_>>();

        for (job, handle) in chunk.iter().zip(handles) {
            let result = handle.join().unwrap_or_else(|_| {
                Err(TunnelRuntimeError::StartWorkerPanic {
                    name: job.resolved.tunnel.name.clone(),
                })
            });
            on_result(job.index, &result);
            results[job.index] = Some(result);
        }
    }

    let mut has_state_changes = false;
    for started in results.iter().filter_map(Option::as_ref).flatten() {
        state_file.upsert(started.state.clone());
        has_state_changes = true;
    }

    if has_state_changes {
        write_state_file(state_path, &state_file)?;
    }

    Ok(results
        .into_iter()
        .map(|result| result.expect("start tunnel result should be recorded"))
        .collect())
}

/// トンネル状態の一覧を取得する
pub fn tunnel_statuses(state_path: &Path) -> Result<Vec<TunnelRuntimeStatus>, TunnelRuntimeError> {
    let state_file = read_state_file(state_path)?;
    let probe = RuntimeStatusProbe::from_states(&state_file.tunnels);

    Ok(tunnel_statuses_from_state_file(state_file, &probe))
}

/// 複数状態ファイルのトンネル状態一覧をまとめて取得する
pub fn tunnel_statuses_for_state_files(
    state_paths: &[&Path],
) -> Result<Vec<Vec<TunnelRuntimeStatus>>, TunnelRuntimeError> {
    let state_files = state_paths
        .iter()
        .map(|path| read_state_file(path))
        .collect::<Result<Vec<_>, _>>()?;
    let probe = RuntimeStatusProbe::from_states(
        state_files
            .iter()
            .flat_map(|state_file| state_file.tunnels.iter()),
    );

    Ok(state_files
        .into_iter()
        .map(|state_file| tunnel_statuses_from_state_file(state_file, &probe))
        .collect())
}

/// 読み込み済み状態ファイルを検査済みのプロセス状態と結合する
fn tunnel_statuses_from_state_file(
    state_file: TunnelStateFile,
    probe: &RuntimeStatusProbe,
) -> Vec<TunnelRuntimeStatus> {
    state_file
        .tunnels
        .into_iter()
        .map(|state| {
            let process_state = probe.process_state_for(&state);

            TunnelRuntimeStatus {
                state,
                process_state,
            }
        })
        .collect()
}

/// トンネルを停止して状態ファイルから削除する
pub fn stop_tunnel(
    runtime_id: &str,
    state_path: &Path,
) -> Result<StoppedTunnel, TunnelRuntimeError> {
    let mut state_file = read_state_file(state_path)?;
    let probe = RuntimeStatusProbe::from_states(&state_file.tunnels);
    let stopped = stop_tunnel_from_state_file(&mut state_file, &probe, runtime_id)?;

    write_state_file(state_path, &state_file)?;

    Ok(stopped)
}

/// 複数トンネルを状態ファイル単位で停止する
pub fn stop_tunnels(
    runtime_ids: &[String],
    state_path: &Path,
) -> Result<Vec<Result<StoppedTunnel, TunnelRuntimeError>>, TunnelRuntimeError> {
    stop_tunnels_with_progress(runtime_ids, state_path, |_index, _result| {})
}

/// 複数トンネルを状態ファイル単位で停止し、各結果確定時に通知する
pub fn stop_tunnels_with_progress<F>(
    runtime_ids: &[String],
    state_path: &Path,
    mut on_result: F,
) -> Result<Vec<Result<StoppedTunnel, TunnelRuntimeError>>, TunnelRuntimeError>
where
    F: FnMut(usize, &Result<StoppedTunnel, TunnelRuntimeError>),
{
    let mut state_file = read_state_file(state_path)?;
    let probe = RuntimeStatusProbe::from_states(&state_file.tunnels);
    let mut results = Vec::with_capacity(runtime_ids.len());
    let mut removal_counts = HashMap::<&str, usize>::new();

    {
        let indexed_states = index_tunnel_states_by_runtime_id(&state_file);
        let mut removed_runtime_ids = HashSet::<&str>::new();

        for (index, runtime_id) in runtime_ids.iter().enumerate() {
            let runtime_id = runtime_id.as_str();
            let result = stop_tunnel_from_indexed_state_file(
                &indexed_states,
                &removed_runtime_ids,
                &probe,
                runtime_id,
            );

            if result.is_ok() {
                removed_runtime_ids.insert(runtime_id);
                *removal_counts.entry(runtime_id).or_default() += 1;
            }

            on_result(index, &result);
            results.push(result);
        }
    }

    if !removal_counts.is_empty() {
        remove_tunnel_states_by_runtime_id_counts(&mut state_file, &mut removal_counts);
        write_state_file(state_path, &state_file)?;
    }

    Ok(results)
}

/// 読み込み済み状態ファイルから指定トンネルを停止して削除する
fn stop_tunnel_from_state_file(
    state_file: &mut TunnelStateFile,
    probe: &RuntimeStatusProbe,
    runtime_id: &str,
) -> Result<StoppedTunnel, TunnelRuntimeError> {
    let Some(state) = state_file.get(runtime_id).cloned() else {
        return Err(TunnelRuntimeError::NotTracked {
            runtime_id: runtime_id.to_owned(),
        });
    };
    let previous_state = probe.process_state_for(&state);

    if previous_state == ProcessState::Running {
        stop_process(&state)?;
    }

    let state = state_file
        .remove(runtime_id)
        .expect("checked tunnel state should remain removable");

    Ok(StoppedTunnel {
        state,
        previous_state,
    })
}

/// 状態ファイル内のトンネルを runtime ID で参照できる索引を生成する
fn index_tunnel_states_by_runtime_id(state_file: &TunnelStateFile) -> HashMap<&str, &TunnelState> {
    let mut states = HashMap::with_capacity(state_file.tunnels.len());

    for state in &state_file.tunnels {
        states.entry(state.runtime_id.as_str()).or_insert(state);
    }

    states
}

/// 索引済み状態ファイルから指定トンネルを停止対象として解決する
fn stop_tunnel_from_indexed_state_file(
    states: &HashMap<&str, &TunnelState>,
    removed_runtime_ids: &HashSet<&str>,
    probe: &RuntimeStatusProbe,
    runtime_id: &str,
) -> Result<StoppedTunnel, TunnelRuntimeError> {
    if removed_runtime_ids.contains(runtime_id) {
        return Err(TunnelRuntimeError::NotTracked {
            runtime_id: runtime_id.to_owned(),
        });
    }

    let Some(state) = states.get(runtime_id).copied() else {
        return Err(TunnelRuntimeError::NotTracked {
            runtime_id: runtime_id.to_owned(),
        });
    };
    let previous_state = probe.process_state_for(state);

    if previous_state == ProcessState::Running {
        stop_process(state)?;
    }

    Ok(StoppedTunnel {
        state: state.clone(),
        previous_state,
    })
}

/// 停止済み runtime ID に対応する状態だけを状態ファイルから削除する
fn remove_tunnel_states_by_runtime_id_counts(
    state_file: &mut TunnelStateFile,
    removal_counts: &mut HashMap<&str, usize>,
) {
    state_file.tunnels.retain(|state| {
        let Some(count) = removal_counts.get_mut(state.runtime_id.as_str()) else {
            return true;
        };

        if *count == 0 {
            return true;
        }

        *count -= 1;
        false
    });
}

/// 設定ファイルパスの正規化結果を再利用して runtime ID を生成する
#[derive(Debug, Default)]
struct RuntimeIdCache {
    normalized_source_paths: HashMap<PathBuf, PathBuf>,
}

impl RuntimeIdCache {
    /// 解決済みトンネル設定から runtime ID を生成する
    fn runtime_id_for_resolved_tunnel(&mut self, resolved: &ResolvedTunnelConfig) -> String {
        tunnel_runtime_id_from_normalized_source_path(
            resolved.source.kind,
            self.normalized_source_path(&resolved.source.path),
            &resolved.tunnel.name,
        )
    }

    /// 設定ファイルパスの正規化結果を取得する
    fn normalized_source_path(&mut self, source_path: &Path) -> &Path {
        self.normalized_source_paths
            .entry(source_path.to_path_buf())
            .or_insert_with(|| normalize_runtime_source_path(source_path))
            .as_path()
    }
}

/// トンネル状態から実行状態を判定する
fn tunnel_process_state(state: &TunnelState) -> ProcessState {
    RuntimeStatusProbe::from_states(std::slice::from_ref(state)).process_state_for(state)
}

/// 状態ファイル内の runtime 検査結果を保持する
#[derive(Debug)]
struct RuntimeStatusProbe {
    pid_is_running: HashMap<u32, bool>,
    local_port_processes: HashMap<u16, LocalPortProcesses>,
}

impl RuntimeStatusProbe {
    /// 状態一覧からプロセス状態の検査結果を初期化する
    fn from_states<'a>(states: impl IntoIterator<Item = &'a TunnelState>) -> Self {
        let mut pid_is_running = HashMap::new();
        let mut running_pids = HashSet::new();
        let mut local_ports = HashSet::new();

        for state in states {
            let is_running = pid_is_running
                .entry(state.pid)
                .or_insert_with(|| process_is_running(state.pid));

            if *is_running {
                running_pids.insert(state.pid);
                local_ports.insert(state.local_port);
            }
        }

        let local_port_processes =
            find_local_port_processes_by_ports_for_pids(&local_ports, &running_pids);

        Self {
            pid_is_running,
            local_port_processes,
        }
    }

    /// 保存済み runtime の現在状態を判定する
    fn process_state_for(&self, state: &TunnelState) -> ProcessState {
        let pid_is_running = self
            .pid_is_running
            .get(&state.pid)
            .copied()
            .unwrap_or(false);
        let Some(local_port_processes) = self.local_port_processes.get(&state.local_port) else {
            return process_state_from_probe(
                state.pid,
                pid_is_running,
                &LocalPortProcesses::default(),
            );
        };

        process_state_from_probe(state.pid, pid_is_running, local_port_processes)
    }
}

/// トンネルPIDとLISTEN状態が一致するかを判定する
fn tunnel_is_running(state: &TunnelState) -> bool {
    tunnel_process_state(state) == ProcessState::Running
}

/// PID存在確認とLISTEN確認から実行状態を判定する
fn process_state_from_probe(
    pid: u32,
    pid_is_running: bool,
    local_port_processes: &LocalPortProcesses,
) -> ProcessState {
    if pid_is_running && local_port_processes.contains_pid(pid) {
        ProcessState::Running
    } else {
        ProcessState::Stale
    }
}

/// SSH 起動コマンドの引数を構築する
pub fn build_ssh_command_args(resolved: &ResolvedTunnelConfig) -> Vec<String> {
    build_ssh_args(resolved)
}

/// SSH 起動引数を構築する
fn build_ssh_args(resolved: &ResolvedTunnelConfig) -> Vec<String> {
    let tunnel = &resolved.tunnel;
    let timeouts = resolved.timeouts;
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
        format!("ConnectTimeout={}", timeouts.connect_timeout_seconds),
        "-o".to_owned(),
        format!(
            "ServerAliveInterval={}",
            timeouts.server_alive_interval_seconds
        ),
        "-o".to_owned(),
        format!("ServerAliveCountMax={}", timeouts.server_alive_count_max),
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

/// 起動後の早期終了確認までの待機時間を取得する
fn start_grace_period(timeouts: ResolvedTimeoutConfig) -> Duration {
    Duration::from_millis(timeouts.start_grace_milliseconds)
}

/// 起動確認でLISTEN状態を待つ上限時間を取得する
fn startup_readiness_wait_period(timeouts: ResolvedTimeoutConfig) -> Duration {
    start_grace_period(timeouts).max(Duration::from_millis(
        STARTUP_LISTEN_CONFIRMATION_MILLISECONDS,
    ))
}

/// 起動状態確認の間隔を取得する
fn startup_probe_interval() -> Duration {
    Duration::from_millis(STARTUP_PROBE_INTERVAL_MILLISECONDS)
}

/// 状態ファイル更新を呼び出し元へ委ねて SSH プロセスを開始する
fn start_tunnel_without_state_write(
    resolved: &ResolvedTunnelConfig,
) -> Result<StartedTunnel, TunnelRuntimeError> {
    ensure_local_endpoint_available(resolved)?;

    let mut child = spawn_ssh_tunnel(resolved)?;
    wait_for_tunnel_readiness(&mut child, resolved)?;

    let started_at_unix_seconds = current_unix_seconds();
    let tunnel_state =
        TunnelState::from_resolved_tunnel(resolved, child.id(), started_at_unix_seconds);

    Ok(StartedTunnel {
        state: tunnel_state,
    })
}

/// SSH プロセスの早期終了と local port のLISTEN状態を確認する
fn wait_for_tunnel_readiness(
    child: &mut Child,
    resolved: &ResolvedTunnelConfig,
) -> Result<(), TunnelRuntimeError> {
    let deadline = Instant::now() + startup_readiness_wait_period(resolved.timeouts);

    loop {
        if let Some(status) =
            child
                .try_wait()
                .map_err(|source| TunnelRuntimeError::StartupCheck {
                    name: resolved.tunnel.name.clone(),
                    source,
                })?
        {
            return Err(TunnelRuntimeError::EarlyExit {
                name: resolved.tunnel.name.clone(),
                status,
            });
        }

        if local_port_is_listening_for_pid(child.id(), resolved.tunnel.local_port) {
            return Ok(());
        }

        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }

        thread::sleep(startup_probe_interval().min(deadline - now));
    }
}

/// 並列開始対象の元位置と設定を保持する
#[derive(Debug, Clone)]
struct StartTunnelJob {
    index: usize,
    resolved: ResolvedTunnelConfig,
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
            name: resolved.tunnel.name.clone(),
            source,
        })
}

/// ローカル側エンドポイントを利用できるかを検証する
fn ensure_local_endpoint_available(
    resolved: &ResolvedTunnelConfig,
) -> Result<(), TunnelRuntimeError> {
    let tunnel = &resolved.tunnel;
    let local_host = tunnel.effective_local_host();
    let local_port = tunnel.local_port;

    TcpListener::bind((local_host, local_port))
        .map(drop)
        .map_err(|source| TunnelRuntimeError::LocalEndpointUnavailable {
            name: tunnel.name.clone(),
            local_host: local_host.to_owned(),
            local_port,
            source,
            processes: find_local_port_processes(local_port),
        })
}

/// ローカルポートを使用している LISTEN プロセスを取得する
fn find_local_port_processes(local_port: u16) -> LocalPortProcesses {
    find_local_port_processes_by_ports(&HashSet::from([local_port]))
        .remove(&local_port)
        .unwrap_or_default()
}

/// 指定PIDが local port をLISTENしているかを判定する
fn local_port_is_listening_for_pid(pid: u32, local_port: u16) -> bool {
    find_local_port_processes_by_ports_for_pids(&HashSet::from([local_port]), &HashSet::from([pid]))
        .remove(&local_port)
        .unwrap_or_default()
        .contains_pid(pid)
}

/// 複数ローカルポートの LISTEN プロセスを 1 回の lsof で取得する
fn find_local_port_processes_by_ports(
    local_ports: &HashSet<u16>,
) -> HashMap<u16, LocalPortProcesses> {
    find_local_port_processes_by_ports_with_pid_filter(local_ports, None)
}

/// 指定PIDが使う複数ローカルポートの LISTEN プロセスを取得する
fn find_local_port_processes_by_ports_for_pids(
    local_ports: &HashSet<u16>,
    pids: &HashSet<u32>,
) -> HashMap<u16, LocalPortProcesses> {
    find_local_port_processes_by_ports_with_pid_filter(local_ports, Some(pids))
}

/// 必要に応じて PID を絞り込んで LISTEN プロセスを取得する
fn find_local_port_processes_by_ports_with_pid_filter(
    local_ports: &HashSet<u16>,
    pids: Option<&HashSet<u32>>,
) -> HashMap<u16, LocalPortProcesses> {
    if local_ports.is_empty() || pids.is_some_and(HashSet::is_empty) {
        return HashMap::new();
    }

    let mut command = Command::new("lsof");
    command.arg("-nP");

    if let Some(pids) = pids {
        command.arg("-a").arg("-p").arg(lsof_pid_list(pids));
    }

    for local_port in local_ports {
        command.arg(format!("-iTCP:{local_port}"));
    }

    command
        .arg("-sTCP:LISTEN")
        .arg("-Fpcn")
        .stdin(Stdio::null())
        .stderr(Stdio::null());

    let output = command.output();

    let Ok(output) = output else {
        return HashMap::new();
    };

    if !output.status.success() {
        return HashMap::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    group_local_port_processes_by_port(parse_lsof_processes(&stdout), local_ports)
}

/// lsof の PID 絞り込み引数を生成する
fn lsof_pid_list(pids: &HashSet<u32>) -> String {
    let mut pids = pids.iter().copied().collect::<Vec<_>>();
    pids.sort_unstable();

    pids.into_iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// lsof の field 出力からプロセス情報を抽出する
fn parse_lsof_processes(output: &str) -> LocalPortProcesses {
    let mut processes = Vec::new();
    let mut current = LocalPortProcessBuilder::default();

    for line in output.lines().filter(|line| !line.is_empty()) {
        let (field, value) = line.split_at(1);

        match field {
            "p" => {
                processes.extend(current.into_processes());
                current = LocalPortProcessBuilder::from_pid(value);
            }
            "c" => current.command = non_empty_string(value),
            "n" => {
                if let Some(endpoint) = non_empty_string(value) {
                    current.endpoints.push(endpoint);
                }
            }
            _ => {}
        }
    }

    processes.extend(current.into_processes());

    LocalPortProcesses::new(processes)
}

/// LISTEN プロセスをローカルポートごとに分類する
fn group_local_port_processes_by_port(
    processes: LocalPortProcesses,
    local_ports: &HashSet<u16>,
) -> HashMap<u16, LocalPortProcesses> {
    let mut grouped = HashMap::<u16, Vec<LocalPortProcess>>::new();

    for process in processes.0 {
        let Some(local_port) = process.endpoint.as_deref().and_then(listen_endpoint_port) else {
            continue;
        };

        if local_ports.contains(&local_port) {
            grouped.entry(local_port).or_default().push(process);
        }
    }

    grouped
        .into_iter()
        .map(|(local_port, processes)| (local_port, LocalPortProcesses::new(processes)))
        .collect()
}

/// lsof の endpoint 表記から LISTEN ポートを抽出する
fn listen_endpoint_port(endpoint: &str) -> Option<u16> {
    let endpoint = endpoint
        .trim()
        .strip_suffix(" (LISTEN)")
        .unwrap_or(endpoint)
        .trim();
    let socket = endpoint
        .rsplit_once(' ')
        .map_or(endpoint, |(_protocol, socket)| socket);
    let (_host, port) = socket.rsplit_once(':')?;

    port.parse().ok()
}

/// 空ではない文字列を保持する
fn non_empty_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

/// lsof field 出力の途中状態を表現する
#[derive(Debug, Default)]
struct LocalPortProcessBuilder {
    pid: Option<u32>,
    command: Option<String>,
    endpoints: Vec<String>,
}

impl LocalPortProcessBuilder {
    /// PID 行からプロセス情報の組み立てを開始する
    fn from_pid(value: &str) -> Self {
        Self {
            pid: value.parse().ok(),
            command: None,
            endpoints: Vec::new(),
        }
    }

    /// 組み立て済みプロセス情報へ変換する
    fn into_processes(self) -> Vec<LocalPortProcess> {
        let Some(pid) = self.pid else {
            return Vec::new();
        };
        let command = self.command.unwrap_or_else(|| "unknown".to_owned());

        if self.endpoints.is_empty() {
            return vec![LocalPortProcess {
                command,
                pid,
                endpoint: None,
            }];
        }

        self.endpoints
            .into_iter()
            .map(|endpoint| LocalPortProcess {
                command: command.clone(),
                pid,
                endpoint: Some(endpoint),
            })
            .collect()
    }
}

/// プロセスが存在するかを判定する
#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };

    // SAFETY: signal 0 はプロセス存在確認のみを行い、メモリ安全性へ影響しない
    unsafe { libc::kill(pid, 0) == 0 }
}

/// プロセスが存在するかを判定する
#[cfg(not(unix))]
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
#[cfg(unix)]
fn stop_process(state: &TunnelState) -> Result<(), TunnelRuntimeError> {
    let pid = libc::pid_t::try_from(state.pid).map_err(|_| TunnelRuntimeError::Stop {
        name: state.name.clone(),
        pid: state.pid,
        source: io::Error::new(
            io::ErrorKind::InvalidInput,
            "pid does not fit platform pid_t",
        ),
    })?;

    // SAFETY: pid_t へ変換済みの PID に終了シグナルを送信するだけで、メモリ安全性へ影響しない
    if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
        return Ok(());
    }

    Err(TunnelRuntimeError::Stop {
        name: state.name.clone(),
        pid: state.pid,
        source: io::Error::last_os_error(),
    })
}

/// プロセスへ終了シグナルを送信する
#[cfg(not(unix))]
fn stop_process(state: &TunnelState) -> Result<(), TunnelRuntimeError> {
    Command::new("kill")
        .arg("-TERM")
        .arg(state.pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| TunnelRuntimeError::Stop {
            name: state.name.clone(),
            pid: state.pid,
            source,
        })
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(TunnelRuntimeError::Stop {
                    name: state.name.clone(),
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
    use std::{net::TcpListener, path::PathBuf, process};

    use crate::{ConfigSource, ConfigSourceKind, TimeoutConfig, TunnelConfig, TunnelStateFile};
    use tempfile::TempDir;

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

    /// SSH 起動引数が解決済みタイムアウト設定から生成されることを検証する
    #[test]
    fn build_ssh_args_uses_timeout_settings() {
        let mut resolved = resolved_tunnel();
        resolved.timeouts = ResolvedTimeoutConfig {
            connect_timeout_seconds: 5,
            server_alive_interval_seconds: 10,
            server_alive_count_max: 2,
            start_grace_milliseconds: 50,
        };

        let args = build_ssh_args(&resolved);

        assert!(args.contains(&"ConnectTimeout=5".to_owned()));
        assert!(args.contains(&"ServerAliveInterval=10".to_owned()));
        assert!(args.contains(&"ServerAliveCountMax=2".to_owned()));
    }

    /// 起動確認待機時間が解決済みタイムアウト設定から生成されることを検証する
    #[test]
    fn start_grace_period_uses_timeout_settings() {
        let timeouts = ResolvedTimeoutConfig {
            connect_timeout_seconds: 5,
            server_alive_interval_seconds: 10,
            server_alive_count_max: 2,
            start_grace_milliseconds: 50,
        };

        let duration = start_grace_period(timeouts);

        assert_eq!(duration, Duration::from_millis(50));
    }

    /// 起動確認待機時間がLISTEN確認用の最小時間を持つことを検証する
    #[test]
    fn startup_readiness_wait_period_has_listen_confirmation_floor() {
        let short_grace = ResolvedTimeoutConfig {
            start_grace_milliseconds: 50,
            ..ResolvedTimeoutConfig::default()
        };
        let long_grace = ResolvedTimeoutConfig {
            start_grace_milliseconds: 1_500,
            ..ResolvedTimeoutConfig::default()
        };

        assert_eq!(
            startup_readiness_wait_period(short_grace),
            Duration::from_millis(STARTUP_LISTEN_CONFIRMATION_MILLISECONDS)
        );
        assert_eq!(
            startup_readiness_wait_period(long_grace),
            Duration::from_millis(1_500)
        );
    }

    /// 一括開始が起動済みトンネルを起動処理なしで報告することを検証する
    #[test]
    fn start_tunnels_reports_already_running_tunnels() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let local_port = listener.local_addr().expect("read listener address").port();
        let resolved = resolved_tunnel_with_local_port(local_port);
        let mut state = TunnelStateFile::new();
        state.upsert(TunnelState::from_resolved_tunnel(
            &resolved,
            process::id(),
            1_700_000_000,
        ));
        write_state_file(&state_path, &state).expect("write state file");

        let results = start_tunnels(&[resolved], &state_path, 4).expect("start tunnels");

        assert_eq!(results.len(), 1);
        assert!(matches!(
            &results[0],
            Err(TunnelRuntimeError::AlreadyRunning { name, .. }) if name == "db"
        ));
    }

    /// 一括開始が結果確定時に進捗通知を呼ぶことを検証する
    #[test]
    fn start_tunnels_reports_progress_for_already_running_tunnels() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let local_port = listener.local_addr().expect("read listener address").port();
        let resolved = resolved_tunnel_with_local_port(local_port);
        let mut state = TunnelStateFile::new();
        state.upsert(TunnelState::from_resolved_tunnel(
            &resolved,
            process::id(),
            1_700_000_000,
        ));
        write_state_file(&state_path, &state).expect("write state file");
        let mut reported_indexes = Vec::new();

        start_tunnels_with_progress(&[resolved], &state_path, 4, |index, _result| {
            reported_indexes.push(index);
        })
        .expect("start tunnels");

        assert_eq!(reported_indexes, vec![0]);
    }

    /// 一括停止が状態ファイル更新と進捗通知を状態ファイル単位で行うことを検証する
    #[test]
    fn stop_tunnels_removes_stale_tunnels_and_reports_progress() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");
        let db = resolved_tunnel();
        let cache = ResolvedTunnelConfig::new(
            ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml")),
            TunnelConfig {
                name: "cache".to_owned(),
                local_port: 25432,
                ..resolved_tunnel().tunnel
            },
        );
        let mut state = TunnelStateFile::new();
        state.upsert(TunnelState::from_resolved_tunnel(
            &db,
            u32::MAX,
            1_700_000_000,
        ));
        state.upsert(TunnelState::from_resolved_tunnel(
            &cache,
            u32::MAX,
            1_700_000_000,
        ));
        write_state_file(&state_path, &state).expect("write state file");
        let runtime_ids = vec![
            runtime_id_for_resolved_tunnel(&db),
            "missing".to_owned(),
            runtime_id_for_resolved_tunnel(&cache),
        ];
        let mut reported_indexes = Vec::new();

        let results = stop_tunnels_with_progress(&runtime_ids, &state_path, |index, _result| {
            reported_indexes.push(index);
        })
        .expect("stop tunnels");

        assert!(matches!(
            &results[0],
            Ok(stopped)
                if stopped.state.name == "db" && stopped.previous_state == ProcessState::Stale
        ));
        assert!(matches!(
            &results[1],
            Err(TunnelRuntimeError::NotTracked { runtime_id }) if runtime_id == "missing"
        ));
        assert!(matches!(
            &results[2],
            Ok(stopped)
                if stopped.state.name == "cache" && stopped.previous_state == ProcessState::Stale
        ));
        assert_eq!(reported_indexes, vec![0, 1, 2]);
        assert!(
            read_state_file(&state_path)
                .expect("read state file")
                .tunnels
                .is_empty()
        );
    }

    /// 一括停止で重複指定された runtime ID は 1 回だけ削除されることを検証する
    #[test]
    fn stop_tunnels_reports_duplicate_runtime_id_as_not_tracked_after_first_stop() {
        let temp_dir = TempDir::new().expect("create state directory");
        let state_path = temp_dir.path().join("state.toml");
        let db = resolved_tunnel();
        let mut state = TunnelStateFile::new();
        state.upsert(TunnelState::from_resolved_tunnel(
            &db,
            u32::MAX,
            1_700_000_000,
        ));
        write_state_file(&state_path, &state).expect("write state file");
        let runtime_id = runtime_id_for_resolved_tunnel(&db);
        let runtime_ids = vec![runtime_id.clone(), runtime_id.clone()];

        let results = stop_tunnels(&runtime_ids, &state_path).expect("stop tunnels");

        assert!(matches!(
            &results[0],
            Ok(stopped)
                if stopped.state.name == "db" && stopped.previous_state == ProcessState::Stale
        ));
        assert!(matches!(
            &results[1],
            Err(TunnelRuntimeError::NotTracked { runtime_id: duplicate }) if duplicate == &runtime_id
        ));
        assert!(
            read_state_file(&state_path)
                .expect("read state file")
                .tunnels
                .is_empty()
        );
    }

    /// LISTEN確認が対象PIDのローカルポートを検出することを検証する
    #[test]
    fn local_port_is_listening_for_pid_detects_current_process_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let local_port = listener.local_addr().expect("read listener address").port();

        assert!(local_port_is_listening_for_pid(process::id(), local_port));
    }

    /// 複数状態ファイルの状態取得が入力順のファイル単位で結果を返すことを検証する
    #[test]
    fn tunnel_statuses_for_state_files_preserves_file_groups() {
        let temp_dir = TempDir::new().expect("create state directory");
        let running_state_path = temp_dir.path().join("running-state.toml");
        let stale_state_path = temp_dir.path().join("stale-state.toml");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let local_port = listener.local_addr().expect("read listener address").port();
        let running_resolved = resolved_tunnel_with_local_port(local_port);
        let stale_resolved = ResolvedTunnelConfig::new(
            ConfigSource::new(ConfigSourceKind::Global, PathBuf::from("global.toml")),
            TunnelConfig {
                name: "cache".to_owned(),
                local_port: 25432,
                ..resolved_tunnel().tunnel
            },
        );
        let mut running_state = TunnelStateFile::new();
        let mut stale_state = TunnelStateFile::new();

        running_state.upsert(TunnelState::from_resolved_tunnel(
            &running_resolved,
            process::id(),
            1_700_000_000,
        ));
        stale_state.upsert(TunnelState::from_resolved_tunnel(
            &stale_resolved,
            u32::MAX,
            1_700_000_000,
        ));
        write_state_file(&running_state_path, &running_state).expect("write running state file");
        write_state_file(&stale_state_path, &stale_state).expect("write stale state file");

        let results = tunnel_statuses_for_state_files(&[
            running_state_path.as_path(),
            stale_state_path.as_path(),
        ])
        .expect("read statuses");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0][0].state.name, "db");
        assert_eq!(results[0][0].process_state, ProcessState::Running);
        assert_eq!(results[1][0].state.name, "cache");
        assert_eq!(results[1][0].process_state, ProcessState::Stale);
    }

    /// PIDが存在し、同じPIDがLISTENしている場合にRunningと判定する
    #[test]
    fn process_state_from_probe_returns_running_for_listening_pid() {
        let pid = 1234;
        let processes = LocalPortProcesses::new(vec![LocalPortProcess {
            command: "ssh".to_owned(),
            pid,
            endpoint: Some("127.0.0.1:15432 (LISTEN)".to_owned()),
        }]);

        let state = process_state_from_probe(pid, true, &processes);

        assert_eq!(state, ProcessState::Running);
    }

    /// PIDが存在しても別PIDだけがLISTENしている場合にStaleと判定する
    #[test]
    fn process_state_from_probe_returns_stale_for_non_listening_pid() {
        let processes = LocalPortProcesses::new(vec![LocalPortProcess {
            command: "postgres".to_owned(),
            pid: 5678,
            endpoint: Some("127.0.0.1:15432 (LISTEN)".to_owned()),
        }]);

        let state = process_state_from_probe(1234, true, &processes);

        assert_eq!(state, ProcessState::Stale);
    }

    /// PIDが存在しない場合にLISTEN状態へ依存せずStaleと判定する
    #[test]
    fn process_state_from_probe_returns_stale_for_missing_pid() {
        let processes = LocalPortProcesses::new(vec![LocalPortProcess {
            command: "ssh".to_owned(),
            pid: 1234,
            endpoint: Some("127.0.0.1:15432 (LISTEN)".to_owned()),
        }]);

        let state = process_state_from_probe(1234, false, &processes);

        assert_eq!(state, ProcessState::Stale);
    }

    /// LISTEN検査結果が空の場合にStaleと判定する
    #[test]
    fn process_state_from_probe_returns_stale_for_empty_listen_probe() {
        let processes = LocalPortProcesses::default();

        let state = process_state_from_probe(1234, true, &processes);

        assert_eq!(state, ProcessState::Stale);
    }

    /// lsof の field 出力から LISTEN プロセス情報を抽出できることを検証する
    #[test]
    fn parse_lsof_processes_reads_field_output() {
        let output = "\
p1234
cssh
n127.0.0.1:15432 (LISTEN)
p5678
cpostgres
n*:15432 (LISTEN)
";

        let processes = parse_lsof_processes(output);

        assert_eq!(
            processes,
            LocalPortProcesses::new(vec![
                LocalPortProcess {
                    command: "ssh".to_owned(),
                    pid: 1234,
                    endpoint: Some("127.0.0.1:15432 (LISTEN)".to_owned()),
                },
                LocalPortProcess {
                    command: "postgres".to_owned(),
                    pid: 5678,
                    endpoint: Some("*:15432 (LISTEN)".to_owned()),
                },
            ])
        );
    }

    /// lsof の同一プロセスに複数 endpoint がある場合に endpoint ごとの行として扱うことを検証する
    #[test]
    fn parse_lsof_processes_reads_multiple_endpoints_for_same_process() {
        let output = "\
p1234
cssh
n127.0.0.1:15432 (LISTEN)
n127.0.0.1:25432 (LISTEN)
";

        let processes = parse_lsof_processes(output);

        assert_eq!(
            processes,
            LocalPortProcesses::new(vec![
                LocalPortProcess {
                    command: "ssh".to_owned(),
                    pid: 1234,
                    endpoint: Some("127.0.0.1:15432 (LISTEN)".to_owned()),
                },
                LocalPortProcess {
                    command: "ssh".to_owned(),
                    pid: 1234,
                    endpoint: Some("127.0.0.1:25432 (LISTEN)".to_owned()),
                },
            ])
        );
    }

    /// lsof の endpoint 表記からローカルポートごとに分類できることを検証する
    #[test]
    fn group_local_port_processes_by_port_extracts_requested_ports() {
        let processes = LocalPortProcesses::new(vec![
            LocalPortProcess {
                command: "ssh".to_owned(),
                pid: 1234,
                endpoint: Some("TCP 127.0.0.1:15432 (LISTEN)".to_owned()),
            },
            LocalPortProcess {
                command: "postgres".to_owned(),
                pid: 5678,
                endpoint: Some("*:5432 (LISTEN)".to_owned()),
            },
        ]);
        let ports = HashSet::from([15432]);

        let grouped = group_local_port_processes_by_port(processes, &ports);

        assert_eq!(
            grouped.get(&15432),
            Some(&LocalPortProcesses::new(vec![LocalPortProcess {
                command: "ssh".to_owned(),
                pid: 1234,
                endpoint: Some("TCP 127.0.0.1:15432 (LISTEN)".to_owned()),
            }]))
        );
        assert!(!grouped.contains_key(&5432));
    }

    /// lsof の PID 絞り込み引数が安定した順序で生成されることを検証する
    #[test]
    fn lsof_pid_list_sorts_pids() {
        let pids = HashSet::from([42, 7, 100]);

        let pid_list = lsof_pid_list(&pids);

        assert_eq!(pid_list, "7,42,100");
    }

    /// ポート使用プロセス情報がエラー表示へ含まれることを検証する
    #[test]
    fn local_endpoint_error_displays_listening_processes() {
        let error = TunnelRuntimeError::LocalEndpointUnavailable {
            name: "db".to_owned(),
            local_host: "127.0.0.1".to_owned(),
            local_port: 15432,
            source: io::Error::new(io::ErrorKind::AddrInUse, "address already in use"),
            processes: LocalPortProcesses::new(vec![LocalPortProcess {
                command: "postgres".to_owned(),
                pid: 5678,
                endpoint: Some("127.0.0.1:15432 (LISTEN)".to_owned()),
            }]),
        };

        let message = error.to_string();

        assert!(message.contains("Local endpoint is not available: db (127.0.0.1:15432)"));
        assert!(message.contains(
            "listening process: postgres (pid: 5678, endpoint: 127.0.0.1:15432 (LISTEN))"
        ));
    }

    /// ポート使用プロセス情報が空の場合に追加表示しないことを検証する
    #[test]
    fn empty_local_port_processes_display_is_empty() {
        let processes = LocalPortProcesses::default();

        assert_eq!(processes.to_string(), "");
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
        resolved_tunnel_with_local_port(15432)
    }

    /// テスト用のローカルポートを指定して統合済みトンネル設定を生成する
    fn resolved_tunnel_with_local_port(local_port: u16) -> ResolvedTunnelConfig {
        ResolvedTunnelConfig::new(
            ConfigSource::new(ConfigSourceKind::Local, PathBuf::from("fwd-deck.toml")),
            TunnelConfig {
                name: "db".to_owned(),
                description: None,
                tags: Vec::new(),
                local_host: Some("127.0.0.1".to_owned()),
                local_port,
                remote_host: "db.internal".to_owned(),
                remote_port: 5432,
                ssh_user: "user".to_owned(),
                ssh_host: "bastion.example.com".to_owned(),
                ssh_port: None,
                identity_file: Some("~/.ssh/id_ed25519".to_owned()),
                timeouts: TimeoutConfig::default(),
            },
        )
    }
}
