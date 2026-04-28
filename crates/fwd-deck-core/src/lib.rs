//! fwd-deck の CLI と将来のアプリで共有する中核機能を提供する。

pub mod config;

pub use config::{
    ConfigLoadError, ConfigPaths, ConfigSource, ConfigSourceKind, EffectiveConfig,
    ResolvedTunnelConfig, TunnelConfig, ValidationError, ValidationReport,
    default_global_config_path, default_local_config_path, load_effective_config, read_config_file,
    validate_config,
};
