#!/usr/bin/env python3
"""Fwd Deckのrelease metadata versionを更新する。"""

from __future__ import annotations

import argparse
import re
from pathlib import Path


VERSION_FILES = [
    Path("crates/fwd-deck-cli/Cargo.toml"),
    Path("crates/fwd-deck-core/Cargo.toml"),
    Path("apps/fwd-deck-app/src-tauri/Cargo.toml"),
    Path("apps/fwd-deck-app/src-tauri/tauri.conf.json"),
    Path("apps/fwd-deck-app/package.json"),
    Path("apps/fwd-deck-app/package-lock.json"),
    Path("Cargo.lock"),
]

PACKAGE_TOML_FILES = [
    Path("crates/fwd-deck-cli/Cargo.toml"),
    Path("crates/fwd-deck-core/Cargo.toml"),
    Path("apps/fwd-deck-app/src-tauri/Cargo.toml"),
]

WORKSPACE_LOCK_PACKAGES = {
    "fwd-deck-app",
    "fwd-deck-cli",
    "fwd-deck-core",
}

SEMVER_PATTERN = re.compile(r"^v?(?P<version>\d+\.\d+\.\d+)$")


def main() -> None:
    """コマンドライン引数に基づいてrelease metadataを更新する。

    Parameters:
        None.

    Returns:
        None.

    Raises:
        SystemExit: 引数または更新対象の検証に失敗した場合に送出する。
    """

    args = parse_args()
    version = normalize_version(args.version)
    repo = args.repo.resolve()

    validate_repo_root(repo)
    updated_files = build_updated_files(repo, version)

    if args.dry_run:
        print(f"dry-run: release metadata would be updated to {version}")
        for path in VERSION_FILES:
            print(path)
        return

    for path, content in updated_files.items():
        path.write_text(content, encoding="utf-8")

    print(f"release metadata updated to {version}")
    for path in VERSION_FILES:
        print(path)


def parse_args() -> argparse.Namespace:
    """CLI引数を解析する。

    Parameters:
        None.

    Returns:
        argparse.Namespace: 解析済み引数を返却する。

    Raises:
        SystemExit: argparseによる検証に失敗した場合に送出する。
    """

    parser = argparse.ArgumentParser(
        description="Update Fwd Deck release metadata versions.",
    )
    parser.add_argument(
        "version",
        help="Release version such as 0.4.0. A leading v is accepted and normalized.",
    )
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path.cwd(),
        help="Repository root. Defaults to the current working directory.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate and print target files without writing changes.",
    )
    return parser.parse_args()


def normalize_version(raw_version: str) -> str:
    """release version表記をファイル用の値へ正規化する。

    Parameters:
        raw_version: ユーザー指定のversion文字列。

    Returns:
        str: 先頭のvを除いたsemantic versionを返却する。

    Raises:
        SystemExit: `X.Y.Z`形式ではない場合に送出する。
    """

    match = SEMVER_PATTERN.match(raw_version)
    if match is None:
        raise SystemExit(f"version must be X.Y.Z or vX.Y.Z: {raw_version}")

    return match.group("version")


def validate_repo_root(repo: Path) -> None:
    """更新対象ファイルが揃ったrepository rootであることを検証する。

    Parameters:
        repo: 検証対象のrepository root。

    Returns:
        None.

    Raises:
        SystemExit: 必須ファイルが存在しない場合に送出する。
    """

    missing_files = [str(path) for path in VERSION_FILES if not (repo / path).is_file()]
    if missing_files:
        raise SystemExit("missing release metadata files:\n" + "\n".join(missing_files))


def build_updated_files(repo: Path, version: str) -> dict[Path, str]:
    """更新後のファイル内容を生成する。

    Parameters:
        repo: repository root。
        version: 設定するrelease version。

    Returns:
        dict[Path, str]: 更新対象の絶対パスと更新後内容の対応を返却する。

    Raises:
        SystemExit: 対象ファイルの構造が想定と異なる場合に送出する。
    """

    updated: dict[Path, str] = {}

    for relative_path in PACKAGE_TOML_FILES:
        path = repo / relative_path
        updated[path] = update_package_toml(path.read_text(encoding="utf-8"), version)

    tauri_config = repo / "apps/fwd-deck-app/src-tauri/tauri.conf.json"
    updated[tauri_config] = update_json_root_version(
        tauri_config.read_text(encoding="utf-8"),
        version,
        tauri_config,
    )

    package_json = repo / "apps/fwd-deck-app/package.json"
    updated[package_json] = update_json_root_version(
        package_json.read_text(encoding="utf-8"),
        version,
        package_json,
    )

    package_lock = repo / "apps/fwd-deck-app/package-lock.json"
    updated[package_lock] = update_package_lock(
        package_lock.read_text(encoding="utf-8"),
        version,
    )

    cargo_lock = repo / "Cargo.lock"
    updated[cargo_lock] = update_cargo_lock(cargo_lock.read_text(encoding="utf-8"), version)

    return updated


def update_package_toml(content: str, version: str) -> str:
    """Cargo package sectionのversionを更新する。

    Parameters:
        content: Cargo.tomlの内容。
        version: 設定するrelease version。

    Returns:
        str: 更新後のCargo.toml内容を返却する。

    Raises:
        SystemExit: package sectionのversion行が特定できない場合に送出する。
    """

    lines = content.splitlines(keepends=True)
    in_package_section = False
    replaced = 0

    for index, line in enumerate(lines):
        if line.strip() == "[package]":
            in_package_section = True
            continue

        if in_package_section and line.startswith("["):
            in_package_section = False

        if in_package_section and line.startswith("version = "):
            newline = "\n" if line.endswith("\n") else ""
            lines[index] = f'version = "{version}"{newline}'
            replaced += 1

    if replaced != 1:
        raise SystemExit(f"expected exactly one package version line, found {replaced}")

    return "".join(lines)


def update_json_root_version(content: str, version: str, path: Path) -> str:
    """root階層のJSON version行だけを更新する。

    Parameters:
        content: 更新対象JSONファイルの内容。
        version: 設定するrelease version。
        path: エラー表示用の更新対象JSONファイル。

    Returns:
        str: 更新後のJSON文字列を返却する。

    Raises:
        SystemExit: root階層のversionが存在しない場合に送出する。
    """

    updated_content, replacements = re.subn(
        r'^(\s{2}"version": ")[^"]+(",?)$',
        rf"\g<1>{version}\2",
        content,
        count=1,
        flags=re.MULTILINE,
    )
    if replacements != 1:
        raise SystemExit(f"missing root version: {path}")

    return updated_content


def update_package_lock(content: str, version: str) -> str:
    """npm package-lockのroot package versionを更新する。

    Parameters:
        content: 更新対象package-lock.jsonの内容。
        version: 設定するrelease version。

    Returns:
        str: 更新後のpackage-lock.json文字列を返却する。

    Raises:
        SystemExit: root package entryが存在しない場合に送出する。
    """

    lines = content.splitlines(keepends=True)
    in_root_package = False
    replaced_top_level = 0
    replaced_root_package = 0

    for index, line in enumerate(lines):
        if line.startswith('    "": {'):
            in_root_package = True
            continue

        if in_root_package and line.startswith("    },"):
            in_root_package = False

        if line.startswith('  "version": '):
            lines[index] = replace_json_version_line(line, version)
            replaced_top_level += 1
            continue

        if in_root_package and line.startswith('      "version": '):
            lines[index] = replace_json_version_line(line, version)
            replaced_root_package += 1

    if replaced_top_level != 1 or replaced_root_package != 1:
        raise SystemExit(
            "expected one top-level and one root package version in package-lock.json"
        )

    return "".join(lines)


def replace_json_version_line(line: str, version: str) -> str:
    """JSON version行の値だけを差し替える。

    Parameters:
        line: JSONのversion行。
        version: 設定するrelease version。

    Returns:
        str: 更新後のversion行を返却する。

    Raises:
        SystemExit: version行の形式が想定と異なる場合に送出する。
    """

    match = re.match(r'^(\s*"version":\s*")[^"]+(")(,?\r?\n?)$', line)
    if match is None:
        raise SystemExit(f"unexpected JSON version line: {line.rstrip()}")

    return f"{match.group(1)}{version}{match.group(2)}{match.group(3)}"


def update_cargo_lock(content: str, version: str) -> str:
    """workspace packageのCargo.lock versionを更新する。

    Parameters:
        content: Cargo.lockの内容。
        version: 設定するrelease version。

    Returns:
        str: 更新後のCargo.lock内容を返却する。

    Raises:
        SystemExit: workspace package entryが不足する場合に送出する。
    """

    blocks = content.split("[[package]]")
    if len(blocks) == 1:
        raise SystemExit("Cargo.lock does not contain package blocks")

    updated_packages: set[str] = set()
    updated_blocks = [blocks[0]]

    for block in blocks[1:]:
        updated_block, package_name = update_cargo_lock_block(block, version)
        if package_name is not None:
            updated_packages.add(package_name)
        updated_blocks.append("[[package]]" + updated_block)

    missing_packages = sorted(WORKSPACE_LOCK_PACKAGES - updated_packages)
    if missing_packages:
        raise SystemExit("missing Cargo.lock package entries:\n" + "\n".join(missing_packages))

    return "".join(updated_blocks)


def update_cargo_lock_block(block: str, version: str) -> tuple[str, str | None]:
    """Cargo.lockの1package blockを必要に応じて更新する。

    Parameters:
        block: `[[package]]`以降のblock文字列。
        version: 設定するrelease version。

    Returns:
        tuple[str, str | None]: 更新後blockと更新対象package名を返却する。

    Raises:
        SystemExit: 更新対象packageにversion行が存在しない場合に送出する。
    """

    name_match = re.search(r'^name = "([^"]+)"$', block, flags=re.MULTILINE)
    if name_match is None:
        return block, None

    package_name = name_match.group(1)
    if package_name not in WORKSPACE_LOCK_PACKAGES:
        return block, None

    updated_block, replacements = re.subn(
        r'^version = "[^"]+"$',
        f'version = "{version}"',
        block,
        count=1,
        flags=re.MULTILINE,
    )
    if replacements != 1:
        raise SystemExit(f"missing Cargo.lock version for package: {package_name}")

    return updated_block, package_name


if __name__ == "__main__":
    main()
