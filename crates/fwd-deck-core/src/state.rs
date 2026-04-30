use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ConfigSourceKind, ResolvedTunnelConfig, path_display::format_path_for_display};

const STATE_FILE_RELATIVE_PATH: &str = ".local/state/fwd-deck/state.toml";

/// 起動中トンネルの状態ファイルを表現する
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TunnelStateFile {
    pub tunnels: Vec<TunnelState>,
}

impl TunnelStateFile {
    /// 空の状態ファイルを初期化する
    pub fn new() -> Self {
        Self {
            tunnels: Vec::new(),
        }
    }

    /// トンネル状態を追加または更新する
    pub fn upsert(&mut self, tunnel: TunnelState) {
        if let Some(existing) = self
            .tunnels
            .iter_mut()
            .find(|item| item.runtime_id == tunnel.runtime_id)
        {
            *existing = tunnel;
            return;
        }

        self.tunnels.push(tunnel);
    }

    /// 指定 runtime ID のトンネル状態を取得する
    pub fn get(&self, runtime_id: &str) -> Option<&TunnelState> {
        self.tunnels
            .iter()
            .find(|item| item.runtime_id == runtime_id)
    }

    /// 指定 runtime ID のトンネル状態を削除する
    pub fn remove(&mut self, runtime_id: &str) -> Option<TunnelState> {
        let position = self
            .tunnels
            .iter()
            .position(|item| item.runtime_id == runtime_id)?;
        Some(self.tunnels.remove(position))
    }

    /// legacy state を含む状態情報を現在の形式へ正規化する
    fn normalize(mut self) -> Self {
        for tunnel in &mut self.tunnels {
            tunnel.normalize_runtime_id();
        }

        self
    }
}

/// 起動中トンネル 1 件の状態を表現する
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TunnelState {
    #[serde(default)]
    pub runtime_id: String,
    #[serde(alias = "id")]
    pub name: String,
    pub pid: u32,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: Option<u16>,
    pub source_kind: ConfigSourceKind,
    pub source_path: PathBuf,
    pub started_at_unix_seconds: u64,
}

impl TunnelState {
    /// 起動結果から状態情報を初期化する
    pub fn from_resolved_tunnel(
        resolved: &ResolvedTunnelConfig,
        pid: u32,
        started_at_unix_seconds: u64,
    ) -> Self {
        let tunnel = &resolved.tunnel;

        Self {
            runtime_id: runtime_id_for_resolved_tunnel(resolved),
            name: tunnel.name.clone(),
            pid,
            local_host: tunnel.effective_local_host().to_owned(),
            local_port: tunnel.local_port,
            remote_host: tunnel.remote_host.clone(),
            remote_port: tunnel.remote_port,
            ssh_user: tunnel.ssh_user.clone(),
            ssh_host: tunnel.ssh_host.clone(),
            ssh_port: tunnel.ssh_port,
            source_kind: resolved.source.kind,
            source_path: resolved.source.path.clone(),
            started_at_unix_seconds,
        }
    }

    /// legacy state に欠けている runtime ID を補完する
    fn normalize_runtime_id(&mut self) {
        if self.runtime_id.is_empty() {
            self.runtime_id = tunnel_runtime_id(self.source_kind, &self.source_path, &self.name);
        }
    }
}

/// 状態ファイル操作時の失敗理由を表現する
#[derive(Debug, Error)]
pub enum StateFileError {
    #[error(
        "Failed to read state file: {}: {source}",
        format_path_for_display(.path)
    )]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Failed to parse state file: {}: {source}",
        format_path_for_display(.path)
    )]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error(
        "Failed to serialize state file: {}: {source}",
        format_path_for_display(.path)
    )]
    Serialize {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error(
        "Failed to create state directory: {}: {source}",
        format_path_for_display(.path)
    )]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Failed to write state file: {}: {source}",
        format_path_for_display(.path)
    )]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// 状態ファイルの既定パスを取得する
pub fn default_state_file_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(STATE_FILE_RELATIVE_PATH))
}

/// 状態ファイルを読み込む
pub fn read_state_file(path: &Path) -> Result<TunnelStateFile, StateFileError> {
    if !path.exists() {
        return Ok(TunnelStateFile::new());
    }

    let content = fs::read_to_string(path).map_err(|source| StateFileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str::<TunnelStateFile>(&content)
        .map(TunnelStateFile::normalize)
        .map_err(|source| StateFileError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

/// 状態ファイルを書き込む
pub fn write_state_file(path: &Path, state: &TunnelStateFile) -> Result<(), StateFileError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| StateFileError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let content = toml::to_string_pretty(state).map_err(|source| StateFileError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;
    fs::write(path, content).map_err(|source| StateFileError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// 解決済み設定から runtime ID を生成する
pub fn runtime_id_for_resolved_tunnel(resolved: &ResolvedTunnelConfig) -> String {
    tunnel_runtime_id(
        resolved.source.kind,
        &resolved.source.path,
        &resolved.tunnel.name,
    )
}

/// runtime 状態の一意な ID を生成する
pub fn tunnel_runtime_id(kind: ConfigSourceKind, source_path: &Path, name: &str) -> String {
    let source_path = normalize_runtime_source_path(source_path);

    format!("{kind}:{}:{name}", source_path.display())
}

/// runtime ID の入力に使う設定ファイルパスを安定化する
fn normalize_runtime_source_path(source_path: &Path) -> PathBuf {
    fs::canonicalize(source_path).unwrap_or_else(|_| source_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::{ConfigSource, TunnelConfig};

    use super::*;

    /// 状態ファイルを TOML として保存し、同じ内容で読み戻せることを検証する
    #[test]
    fn state_file_round_trips_as_toml() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let state_path = temp_dir.path().join("state.toml");
        let mut state = TunnelStateFile::new();
        state.upsert(tunnel_state("db", 1000));

        write_state_file(&state_path, &state).expect("write state file");
        let loaded = read_state_file(&state_path).expect("read state file");

        assert_eq!(loaded, state);
    }

    /// 存在しない状態ファイルが空の状態として扱われることを検証する
    #[test]
    fn missing_state_file_returns_empty_state() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let state_path = temp_dir.path().join("missing.toml");

        let loaded = read_state_file(&state_path).expect("read missing state file");

        assert!(loaded.tunnels.is_empty());
    }

    /// legacy state の id が name として読み込まれることを検証する
    #[test]
    fn legacy_state_id_is_read_as_name() {
        let temp_dir = TempDir::new().expect("create a temporary directory");
        let state_path = temp_dir.path().join("state.toml");
        fs::write(
            &state_path,
            r#"
[[tunnels]]
id = "db"
pid = 1000
local_host = "127.0.0.1"
local_port = 15432
remote_host = "127.0.0.1"
remote_port = 5432
ssh_user = "user"
ssh_host = "bastion.example.com"
source_kind = "local"
source_path = "fwd-deck.toml"
started_at_unix_seconds = 1700000000
"#,
        )
        .expect("write legacy state file");

        let loaded = read_state_file(&state_path).expect("read legacy state file");

        assert_eq!(loaded.tunnels[0].name, "db");
        assert!(!loaded.tunnels[0].runtime_id.is_empty());
    }

    /// 同じ runtime ID の状態が追加ではなく更新されることを検証する
    #[test]
    fn upsert_replaces_existing_tunnel_state() {
        let mut state = TunnelStateFile::new();
        state.upsert(tunnel_state("db", 1000));
        state.upsert(tunnel_state("db", 2000));

        assert_eq!(state.tunnels.len(), 1);
        assert_eq!(state.tunnels[0].pid, 2000);
    }

    /// 同じ name でも source が異なる状態が共存できることを検証する
    #[test]
    fn upsert_keeps_same_name_with_different_sources() {
        let mut state = TunnelStateFile::new();
        state.upsert(tunnel_state_with_source(
            "db",
            ConfigSourceKind::Global,
            PathBuf::from("global.toml"),
            1000,
        ));
        state.upsert(tunnel_state_with_source(
            "db",
            ConfigSourceKind::Local,
            PathBuf::from("fwd-deck.toml"),
            2000,
        ));

        assert_eq!(state.tunnels.len(), 2);
        assert_eq!(state.tunnels[0].name, "db");
        assert_eq!(state.tunnels[1].name, "db");
        assert_ne!(state.tunnels[0].runtime_id, state.tunnels[1].runtime_id);
    }

    /// テスト用の状態情報を生成する
    fn tunnel_state(name: &str, pid: u32) -> TunnelState {
        tunnel_state_with_source(
            name,
            ConfigSourceKind::Local,
            PathBuf::from("fwd-deck.toml"),
            pid,
        )
    }

    /// テスト用の状態情報を source 指定で生成する
    fn tunnel_state_with_source(
        name: &str,
        source_kind: ConfigSourceKind,
        source_path: PathBuf,
        pid: u32,
    ) -> TunnelState {
        let resolved = ResolvedTunnelConfig::new(
            ConfigSource::new(source_kind, source_path),
            TunnelConfig {
                name: name.to_owned(),
                description: None,
                tags: Vec::new(),
                local_host: None,
                local_port: 15432,
                remote_host: "127.0.0.1".to_owned(),
                remote_port: 5432,
                ssh_user: "user".to_owned(),
                ssh_host: "bastion.example.com".to_owned(),
                ssh_port: None,
                identity_file: None,
                timeouts: crate::TimeoutConfig::default(),
            },
        );

        TunnelState::from_resolved_tunnel(&resolved, pid, 1_700_000_000)
    }
}
