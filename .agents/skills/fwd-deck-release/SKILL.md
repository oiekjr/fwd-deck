---
name: fwd-deck-release
description: Prepare normal current-tip Fwd Deck releases by determining the next date-based version when omitted, then updating fixed Cargo, npm, Tauri, and lockfile version metadata. Use when Codex needs to bump the Fwd Deck release version, create a release preparation commit, tag a new release, or verify release version consistency; do not use for backfilling releases into past commits or rewriting published history.
---

# Fwd Deck Release

## Scope

Use this skill only for a normal release prepared at the current branch tip.
If the user wants to insert release commits into older history, retag an existing release, or rewrite remote history, handle that as a separate git operation and ask for explicit confirmation before destructive or force-push actions.

## Version Update

If the user does not specify a target version, determine the next version from the Asia/Tokyo date and local release tags before editing files.
Ask for the exact version only when the user wants a non-default date basis, local tags are ambiguous, or the next version cannot be determined reliably.

Release versions use `YY.MDD.N` as a SemVer-compatible three-number format:

- `YY` is the last two digits of the year.
- `MDD` is the month without zero padding followed by a two-digit day.
- `N` is the release sequence number for the same date.

For the first release on a date, use `YY.MDD.1`.
For additional releases on the same date, increment only `N`.
Use the Asia/Tokyo date unless the user explicitly specifies another date basis.
The bundled script calculates this default from local tags matching `vYY.MDD.N`; it does not fetch remote tags.
Ask before running `git fetch --tags` or any other remote check.

Run the bundled script from the repository root without a version when the user omitted it:

```sh
python3 .agents/skills/fwd-deck-release/scripts/prepare_release.py
```

When the user specifies a version, pass it explicitly:

```sh
python3 .agents/skills/fwd-deck-release/scripts/prepare_release.py 26.616.1
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
2. Determine the target version:
   - If the user specified a version, confirm it is a plain `YY.MDD.N` version such as `26.616.1`; normalize `v26.616.1` to `26.616.1` only for file updates.
   - If the user omitted the version, run `scripts/prepare_release.py --dry-run` to calculate and report the next version from local tags.
3. Run `scripts/prepare_release.py` with no version for the default next version, or `scripts/prepare_release.py <version>` for an explicit target, to update version metadata.
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
