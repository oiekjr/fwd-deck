---
name: fwd-deck-release
description: Prepare normal current-tip Fwd Deck releases by updating fixed Cargo, npm, Tauri, and lockfile version metadata. Use when Codex needs to bump the Fwd Deck release version, create a release preparation commit, tag a new release, or verify release version consistency; do not use for backfilling releases into past commits or rewriting published history.
---

# Fwd Deck Release

## Scope

Use this skill only for a normal release prepared at the current branch tip.
If the user wants to insert release commits into older history, retag an existing release, or rewrite remote history, handle that as a separate git operation and ask for explicit confirmation before destructive or force-push actions.

## Version Update

If the user does not specify a target version, ask the user for the exact version before editing files.
Do not infer a default next version from tags, package metadata, or commit history.

Run the bundled script from the repository root:

```sh
python3 .agents/skills/fwd-deck-release/scripts/prepare_release.py 0.4.0
```

The script updates only these release metadata files:

```text
crates/fwd-deck-cli/Cargo.toml
crates/fwd-deck-core/Cargo.toml
apps/fwd-deck-app/src-tauri/Cargo.toml
apps/fwd-deck-app/src-tauri/tauri.conf.json
apps/fwd-deck-app/package.json
apps/fwd-deck-app/package-lock.json
Cargo.lock
```

Use `--dry-run` first when the worktree has unrelated user edits or when checking the next version without changing files.

## Workflow

1. Inspect `git status --short --branch` and preserve unrelated user changes.
2. Confirm the target version is a plain semantic version such as `0.4.0`; if the user omitted it, ask for the exact version before proceeding.
   Normalize `v0.4.0` to `0.4.0` only for file updates.
3. Run `scripts/prepare_release.py <version>` to update version metadata.
4. Verify release workflow inputs:

```sh
cargo pkgid -p fwd-deck-cli
cargo pkgid -p fwd-deck-app
node -p "require('./apps/fwd-deck-app/package.json').version + ' ' + require('./apps/fwd-deck-app/src-tauri/tauri.conf.json').version"
```

5. Run required project checks before committing:

```sh
task fmt
task app:format
task check
```

6. Stage only the release metadata files unless formatting changed additional files that are part of the requested release work.
7. Commit with this Japanese Conventional Commit message:

```text
chore(release): <version>リリース準備を行う
```

8. Create the local release tag only after checks pass:

```sh
git tag v<version>
```

9. Push only when the user explicitly asks for it:

```sh
git push origin HEAD
git push origin v<version>
```

## Failure Handling

If the script reports missing files, missing package entries, or inconsistent lockfile contents, stop and inspect the repository layout before editing manually.
If `task app:format` changes unrelated user edits, report the affected files and do not revert them unless the user asks.
