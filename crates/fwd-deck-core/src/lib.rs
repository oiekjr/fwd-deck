//! fwd-deck の CLI と将来のアプリで共有する中核機能を提供する。

pub mod config;
pub mod state;
pub mod tunnel;

pub use config::{
    ConfigEditError, ConfigLoadError, ConfigPaths, ConfigSource, ConfigSourceKind,
    DEFAULT_LOCAL_HOST, EffectiveConfig, ResolvedTunnelConfig, TunnelConfig, ValidationError,
    ValidationReport, ValidationWarning, add_tunnel_to_config_file, default_global_config_path,
    default_local_config_path, filter_tunnels_by_tags, load_effective_config, normalize_tag,
    normalize_tags, read_config_file, remove_tunnel_from_config_file, tag_is_valid,
    tunnel_matches_tags, validate_config,
};
pub use state::{StateFileError, TunnelState, TunnelStateFile, default_state_file_path};
pub use tunnel::{
    ProcessState, StartedTunnel, StoppedTunnel, TunnelRuntimeError, TunnelRuntimeStatus,
    start_tunnel, stop_tunnel, tunnel_statuses,
};
