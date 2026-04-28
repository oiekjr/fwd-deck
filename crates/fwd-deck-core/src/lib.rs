//! fwd-deck の CLI と将来のアプリで共有する中核機能を提供する。

pub mod config;
pub mod state;
pub mod tunnel;

pub use config::{
    ConfigLoadError, ConfigPaths, ConfigSource, ConfigSourceKind, EffectiveConfig,
    ResolvedTunnelConfig, TunnelConfig, ValidationError, ValidationReport,
    default_global_config_path, default_local_config_path, load_effective_config, read_config_file,
    validate_config,
};
pub use state::{StateFileError, TunnelState, TunnelStateFile, default_state_file_path};
pub use tunnel::{
    ProcessState, StartedTunnel, StoppedTunnel, TunnelRuntimeError, TunnelRuntimeStatus,
    start_tunnel, stop_tunnel, tunnel_statuses,
};
