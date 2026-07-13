//! fwd-deck の CLI と将来のアプリで共有する中核機能を提供する。

pub mod config;
pub mod path_display;
pub mod ssh_import;
pub mod state;
pub mod tunnel;

pub use config::{
    ConfigEditError, ConfigLoadError, ConfigPaths, ConfigSource, ConfigSourceKind,
    DEFAULT_CONNECT_TIMEOUT_SECONDS, DEFAULT_LOCAL_HOST, DEFAULT_SERVER_ALIVE_COUNT_MAX,
    DEFAULT_SERVER_ALIVE_INTERVAL_SECONDS, DEFAULT_START_GRACE_MILLISECONDS, EffectiveConfig,
    LoadedConfigFile, LoadedConfigOverrideFile, ResolvedTimeoutConfig, ResolvedTunnelConfig,
    TimeoutConfig, TunnelConfig, TunnelConfigOverride, ValidationError, ValidationReport,
    ValidationWarning, add_tunnel_to_config_file, add_tunnels_to_config_file,
    default_global_config_path, default_local_config_path, filter_tunnels_by_tags,
    filter_tunnels_excluding_tags, load_effective_config, local_override_config_path,
    normalize_tag, normalize_tags, read_config_file, read_config_override_file,
    remove_tunnel_and_override_from_config_files, remove_tunnel_from_config_file,
    remove_tunnel_override_from_config_file, tag_is_valid, tunnel_matches_tags,
    update_tunnel_and_override_name_in_config_files, update_tunnel_in_config_file,
    upsert_tunnel_override_in_config_file, validate_config,
};
pub use path_display::{format_path_for_display, format_path_for_display_with_home};
pub use ssh_import::{
    DEFAULT_IMPORT_TAG, SshImport, SshImportError, SshImportWarning, SshLocalForward,
    generate_import_tunnel_name, parse_ssh_args, parse_ssh_command,
};
pub use state::{
    StateFileError, TunnelState, TunnelStateFile, default_state_file_path,
    normalize_runtime_source_path, runtime_id_for_resolved_tunnel, tunnel_runtime_id,
    tunnel_runtime_id_from_normalized_source_path,
};
pub use tunnel::{
    ProcessState, StartTunnelOptions, StartedTunnel, StoppedTunnel, TunnelRuntimeError,
    TunnelRuntimeStatus, build_ssh_command_args, start_tunnel, start_tunnel_with_options,
    start_tunnels, start_tunnels_with_options, start_tunnels_with_progress, stop_tunnel,
    stop_tunnels, stop_tunnels_with_progress, tunnel_statuses, tunnel_statuses_for_state_files,
};
