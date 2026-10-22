---
name: fwd-deck-cli
description: Configure and operate named SSH local port forwarding tunnels with the fwd-deck CLI. Use when asked to manage tunnel configuration, import SSH commands, start or stop tunnels, inspect status, recover connections, or diagnose fwd-deck CLI problems. Does not cover developing or releasing fwd-deck itself.
---

# Fwd Deck CLI

## Requirements and command discovery

Use the installed `fwd-deck` executable, OpenSSH (`ssh`) for tunnels, and `lsof` for local listener checks.
On macOS, the CLI can be installed with `brew install oiekjr/tap/fwd-deck`.

Check the installed CLI before using unfamiliar options:

```sh
fwd-deck --help
fwd-deck start --help
fwd-deck config import-ssh --help
```

`fwd-deck --version` also checks GitHub Releases for an update; it is not an offline version check.

## Configuration and scope

Run commands from the intended project's directory. Local configuration is resolved from the current working directory, not by searching parent directories.

| File | Default path | Purpose |
| --- | --- | --- |
| Local configuration | `./fwd-deck.toml` | Project settings that can be shared in Git |
| Local override | `./fwd-deck.override.toml` | Personal changes to local settings |
| Global configuration | `~/.config/fwd-deck/config.toml` | Settings available across projects |
| Runtime state | `~/.local/state/fwd-deck/state.toml` | Tracks tunnel processes for status, stop, recover, and watch |

- `--config PATH` selects the local configuration. Its override is a sibling file: `custom.toml` uses `custom.override.toml`.
- `--global-config PATH` selects the global configuration; `--no-global` disables loading global configuration.
- `--state PATH` selects a different runtime state file. Use the same state path when later inspecting or stopping those tunnels. Let the CLI manage the state file.
- Local and global tunnels can share a name. Named operations prefer local tunnels unless `--scope global` is specified. `show`, `start`, `stop`, `recover`, and `watch` accept `--scope local|global`; `list`, `status`, and `validate` do not.
- Runtime state can contain tunnels from multiple projects. Inspect the target and scope before stopping or recovering tunnels; do not broaden a named request to `--all`.

### Create or change settings

Use the user's actual endpoints and ports in place of the example values:

```toml
[timeouts]
connect_timeout_seconds = 15
server_alive_interval_seconds = 30
server_alive_count_max = 3
start_grace_milliseconds = 300

[[tunnels]]
name = "dev-db"
enabled = true
description = "Development database"
tags = ["dev", "project-a"]
local_host = "127.0.0.1"
local_port = 15432
remote_host = "db.example.com"
remote_port = 5432
ssh_user = "ec2-user"
ssh_host = "bastion.example.com"
ssh_port = 22
identity_file = "~/.ssh/id_ed25519"

[tunnels.timeouts]
connect_timeout_seconds = 10
```

Required fields are `name`, `local_port`, `remote_host`, `remote_port`, `ssh_user`, and `ssh_host`.
`remote_host` is reached from the SSH server. `local_host` defaults to `127.0.0.1`, `enabled` defaults to `true`, and an omitted `ssh_port` uses SSH's default.
Top-level timeouts provide defaults; `[tunnels.timeouts]` overrides them for one tunnel.

Names and local ports must be unique within each configuration file, including disabled tunnels.
Duplicates across local and global files are allowed, but tunnels cannot simultaneously bind the same local endpoint.
Tags use lowercase ASCII letters, digits, `-`, `_`, `.`, and `/`.
Tunnels with `enabled = false` are excluded from normal listing, lookup, and startup.

`config add`, `config edit NAME`, and `config remove` prompt for input even when `--scope` is provided.
For noninteractive changes, edit the appropriate TOML file directly, preserve unrelated settings, and run `validate` afterward.
These interactive commands edit the shared configuration, not personal overrides; timeout changes also require direct TOML editing.

### Personal overrides

Prefer a local override for machine-specific differences to a shared local configuration, and exclude the override file from Git:

```toml
[[tunnels]]
name = "dev-db"
local_port = 25432
```

An override matches an existing local tunnel by `name`. Omitted fields inherit the base value; arrays such as `tags` replace the base array, so `tags = []` clears it.
Overrides can change tunnel fields and per-tunnel timeouts, but cannot rename or add tunnels, change global settings, or set top-level `[timeouts]`.
An unmatched name produces a warning and is ignored. Duplicate names, unknown fields, and invalid value types are errors.

### Import an existing SSH command

```sh
fwd-deck config import-ssh --scope local --name dev-db -- \
  ssh -N -L 15432:db.example.com:5432 ec2-user@bastion.example.com
```

Import parses the command and writes settings; it does not execute SSH or start a tunnel.
Specify `--scope local` or `--scope global` to avoid a scope prompt.
For multiple `-L` forwards, use `--name-prefix db` instead of `--name`; this generates `db-1`, `db-2`, and so on.
Imports receive an `imported` tag by default; use `--no-import-tag` to omit it or repeat `--tag TAG` to add tags.
Review warnings about options that cannot be preserved. Remote forwarding (`-R`), dynamic forwarding (`-D`), and Unix socket forwarding are unsupported.

## Inspect and operate tunnels

First discover the intended tunnel, inspect its effective settings, validate, and preview startup:

```sh
fwd-deck --json list
fwd-deck --json list --tag dev --query db
fwd-deck --json show dev-db --scope local
fwd-deck --json validate
fwd-deck --json start dev-db --scope local --dry-run
```

`list --query TEXT` searches names and descriptions without case sensitivity. Repeated `--tag` filters require all specified tags.
`show` displays effective settings, including applied overrides and configuration sources.

Run only the lifecycle operations needed for the user's request:

```sh
fwd-deck start dev-db --scope local
fwd-deck --json status
fwd-deck stop dev-db --scope local --dry-run
fwd-deck stop dev-db --scope local
```

- `start` and `stop` without names or a selector prompt for tunnel selection. Supply explicit targets for noninteractive use.
- For bulk startup, choose names, `--tag`, or `--all`; these selectors cannot be combined. `--exclude-tag` requires `--all` and excludes tunnels matching any of its tags. Preview the selection before starting it.
- Startup parallelism defaults to 4 and can be changed with `--parallel N`, where `N` must be at least 1.
- `stop --all` acts on tracked tunnels. It cannot be combined with names.
- Startup and stop dry-runs neither change processes nor update runtime state. A successful preview does not mean a connection has been established.
- Startup detaches SSH from the caller's process group by default. Use `--no-detach` only when the caller must retain that process-group relationship.
- Starting an already-running tracked tunnel succeeds and reports its PID. Use `status` to verify the result; a tracked process does not prove the remote application is healthy.

### Recovery and monitoring

```sh
fwd-deck recover dev-db --scope local
fwd-deck watch dev-db --scope local --interval-seconds 5
```

`recover` restarts stale tracked tunnels using current settings. A tunnel is stale when its tracked process is missing or is no longer listening on the configured local endpoint.
Without names, `recover` considers all stale tracked tunnels; `watch` monitors all tracked tunnels.
`watch` runs continuously in the foreground and can reconnect tunnels. Use it only when continuous monitoring is requested and manage its lifetime explicitly.

## JSON output and failures

Use `--json` only with `list`, `show`, `status`, `validate`, and `start --dry-run`.
Do not add it to actual startup, stop, recover, watch, configuration edits, doctor, completion, or open.
JSON uses camelCase field names, such as `localPort` and `overridePath`; `overridePath` is `null` when no override applies.

Check the exit code and standard error as well as standard output. Failures are not guaranteed to be JSON, and a successful exit can represent an empty selection.
Confirm that the output includes the intended target before reporting that an operation succeeded.
If an expected tunnel is missing, check the working directory, selected files, scope, filters, and `enabled` setting before retrying.

## Diagnostics and other commands

- `fwd-deck doctor` checks configuration, state-file access, `ssh` and `lsof`, identity-file existence, and local endpoint usage. Its state-file writability check can create a temporary probe file; it is not a purely read-only check.
- For a local port conflict, inspect the owning listener or choose an appropriate override port rather than stopping an unrelated process.
- `fwd-deck completion zsh` prints a completion script. Supported shells are `bash`, `elvish`, `fish`, `powershell`, and `zsh`.
- `fwd-deck open [PATH]` requires macOS and the separately installed Fwd Deck app. It opens an existing directory as the app's workspace, defaulting to the current directory. Switching an existing app's workspace stops the old workspace's local tunnels and keeps global tunnels.
