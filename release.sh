#!/usr/bin/env bash
set -Eeuo pipefail

readonly SCRIPT_NAME=${0##*/}
readonly TARGET=x86_64-unknown-linux-musl
readonly BASE_IMAGE=rust:1.90-alpine
readonly BUILD_IMAGE=law-of-cycles-release-rust:1.90-alpine-musl
readonly ARCHIVE_NAME=portable-kami-amd64.zip

release_version=
tag=
branch=
repo_root=
repo_url=
work_dir=
output_dir=
files_changed=false
files_staged=false
committed=false
output_created=false

usage() {
    cat <<EOF
Usage: $SCRIPT_NAME --version VERSION

Create a local kami release. VERSION uses three numeric components, such as 0.1.0.
The script builds and tests a static Linux amd64 binary, writes a ZIP and
SHA-256 file under dist/, commits version changes, and creates an annotated tag.
It prints the GitHub release fields and push commands; it does not push.
EOF
}

die() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

cleanup() {
    local status=$?

    if [[ $status -ne 0 && $committed == false ]]; then
        if [[ $files_staged == true ]]; then
            git -C "$repo_root" restore --staged -- release.sh Cargo.toml Cargo.lock src/backend.rs tests/rust_regression.rs || true
        fi
        if [[ $files_changed == true ]]; then
            cp -- "$work_dir/Cargo.toml" "$repo_root/Cargo.toml"
            cp -- "$work_dir/Cargo.lock" "$repo_root/Cargo.lock"
            cp -- "$work_dir/backend.rs" "$repo_root/src/backend.rs"
            cp -- "$work_dir/rust_regression.rs" "$repo_root/tests/rust_regression.rs"
        fi
        if [[ $output_created == true && -d $output_dir ]]; then
            rm -r -- "$output_dir"
        fi
    fi
    if [[ -n $work_dir && -d $work_dir ]]; then
        rm -r -- "$work_dir"
    fi
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
        --version)
            [[ $# -ge 2 && -z $release_version ]] || die 'use --version once with a value'
            release_version=$2
            shift 2
            ;;
        --help | -h)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
        esac
    done

    [[ $release_version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] ||
        die 'version must look like 0.1.0 or 1.0.0'
    tag=$release_version
}

check_repository() {
    local status_line origin
    repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
    [[ $(git -C "$repo_root" rev-parse --show-toplevel) == "$repo_root" ]] ||
        die 'release.sh must be at the Git repository root'
    branch=$(git -C "$repo_root" symbolic-ref --quiet --short HEAD) ||
        die 'release requires a checked-out branch'
    git -C "$repo_root" diff --cached --quiet || die 'staged changes must be committed first'

    while IFS= read -r status_line; do
        case "$status_line" in
        '?? release.sh' | ' M release.sh') ;;
        *) die "working tree must be clean except for release.sh: $status_line" ;;
        esac
    done < <(git -C "$repo_root" status --porcelain=v1 --untracked-files=all)

    if git -C "$repo_root" rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
        die "tag already exists: $tag"
    fi
    origin=$(git -C "$repo_root" remote get-url origin) || die 'origin remote is required'
    case "$origin" in
    https://github.com/*) repo_url=${origin%.git} ;;
    git@github.com:*)
        repo_url="https://github.com/${origin#git@github.com:}"
        repo_url=${repo_url%.git}
        ;;
    *) die "cannot make a GitHub release URL from origin: $origin" ;;
    esac
    output_dir="$repo_root/dist/$tag"
    [[ ! -e $output_dir ]] || die "output directory already exists: $output_dir"
}

update_versions() {
    cp -- "$repo_root/Cargo.toml" "$work_dir/Cargo.toml"
    cp -- "$repo_root/Cargo.lock" "$work_dir/Cargo.lock"
    cp -- "$repo_root/src/backend.rs" "$work_dir/backend.rs"
    cp -- "$repo_root/tests/rust_regression.rs" "$work_dir/rust_regression.rs"
    files_changed=true

    (
        cd -- "$repo_root" && python3 - "$release_version" <<'PY'
from pathlib import Path
import re
import sys

new_version = sys.argv[1]


def package_version(text: str) -> str:
    package = re.search(r"(?ms)^\[package\]\n(.*?)(?=^\[|\Z)", text)
    if package is None:
        raise SystemExit("Cargo.toml has no [package] section")
    version = re.search(r'^version = "([^"]+)"$', package.group(1), re.M)
    if version is None:
        raise SystemExit("Cargo.toml has no package version")
    return version.group(1)


def replace_once(path: Path, old: str, new: str) -> None:
    contents = path.read_text()
    if contents.count(old) != 1:
        raise SystemExit(f"expected exactly one {old!r} in {path}")
    path.write_text(contents.replace(old, new))


manifest = Path("Cargo.toml")
old_version = package_version(manifest.read_text())
if old_version == new_version:
    raise SystemExit(f"package version is already {new_version}")

lock = Path("Cargo.lock")
lock_text = lock.read_text()
packages = re.split(r"(?=^\[\[package\]\]$)", lock_text, flags=re.M)
own_packages = [block for block in packages if re.search(r'^name = "law-of-cycles"$', block, re.M)]
if len(own_packages) != 1 or f'version = "{old_version}"' not in own_packages[0]:
    raise SystemExit("Cargo.lock package version does not match Cargo.toml")

replace_once(manifest, f'version = "{old_version}"', f'version = "{new_version}"')
replace_once(lock, own_packages[0], own_packages[0].replace(
    f'version = "{old_version}"', f'version = "{new_version}"', 1
))
replace_once(
    Path("src/backend.rs"),
    f'.user_agent("kami/{old_version}")',
    f'.user_agent("kami/{new_version}")',
)
replace_once(
    Path("tests/rust_regression.rs"),
    f'.contains("{old_version}")',
    f'.contains("{new_version}")',
)
PY
    )
}

build_release() {
    printf 'Preparing %s from %s\n' "$BUILD_IMAGE" "$BASE_IMAGE"
    docker build --quiet --platform linux/amd64 --tag "$BUILD_IMAGE" - <<'DOCKERFILE' >"$work_dir/build-image.txt"
FROM rust:1.90-alpine
RUN apk add --no-cache musl-dev
DOCKERFILE

    printf 'Testing and building %s for %s with %s\n' "$release_version" "$TARGET" "$BUILD_IMAGE"
    docker run --rm --platform linux/amd64 \
        --user "$(id -u):$(id -g)" \
        -e HOME=/app/.devhome -e CARGO_HOME=/app/.devhome/cargo \
        -v "$repo_root:/app" -w /app "$BUILD_IMAGE" \
        sh -ec 'mkdir -p /app/.devhome/cargo && cargo test --locked --target x86_64-unknown-linux-musl && cargo build --release --locked --target x86_64-unknown-linux-musl'

    local binary="$repo_root/target/$TARGET/release/kami"
    [[ -f $binary && -x $binary ]] || die "build did not create an executable: $binary"
    readelf -l "$binary" >"$work_dir/elf-program-headers.txt"
    if grep -q 'INTERP' "$work_dir/elf-program-headers.txt"; then
        die 'built binary has a dynamic interpreter and is not portable'
    fi
    cp -- "$binary" "$work_dir/kami"
    chmod 755 "$work_dir/kami"
}

package_release() {
    local checksum
    mkdir -p -- "$output_dir"
    output_created=true
    (cd -- "$work_dir" && python3 -m zipfile -c "$output_dir/$ARCHIVE_NAME" kami)
    python3 -m zipfile -t "$output_dir/$ARCHIVE_NAME"
    (cd -- "$output_dir" && sha256sum "$ARCHIVE_NAME" >"$ARCHIVE_NAME.sha256")
    (cd -- "$output_dir" && sha256sum --check "$ARCHIVE_NAME.sha256")
    checksum=$(cut -d ' ' -f 1 "$output_dir/$ARCHIVE_NAME.sha256")

    cat >"$output_dir/release-notes.md" <<EOF
## kami $release_version

Release of the Linux terminal client for an existing Mihomo controller.

- Four-page TUI and command-line operations for status, proxies, connections, and logs.
- Node selection and delay checks, runtime mode and TUN controls, and local systemd actions.
- Portable static Linux amd64 binary; no Mihomo core is bundled.

Download $ARCHIVE_NAME and extract kami. Configure a Mihomo controller before connecting; kami tui --demo can be tried without one.

SHA-256 ($ARCHIVE_NAME): $checksum
EOF
}

commit_and_tag() {
    git -C "$repo_root" add -- release.sh Cargo.toml Cargo.lock src/backend.rs tests/rust_regression.rs
    files_staged=true
    git -C "$repo_root" commit -m "chore: release $tag"
    committed=true
    git -C "$repo_root" tag -a "$tag" -m "kami $release_version"
}

print_release_info() {
    printf '\nRelease created locally at commit %s\n' "$(git -C "$repo_root" rev-parse --short HEAD)"
    printf 'Push first:\n  git push origin %s\n  git push origin %s\n' "$branch" "$tag"
    printf '\nNew release: %s/releases/new?tag=%s&target=%s&title=%s\n' \
        "$repo_url" "$tag" "$branch" "$release_version"
    printf 'Tag: %s\nTarget: %s\nTitle: %s\nPre-release: no\n' \
        "$tag" "$branch" "$release_version"
    printf 'Upload: %s\nUpload: %s\n' \
        "$output_dir/$ARCHIVE_NAME" "$output_dir/$ARCHIVE_NAME.sha256"
    printf '\nDescription (copy from %s):\n\n' "$output_dir/release-notes.md"
    cat -- "$output_dir/release-notes.md"
}

main() {
    parse_args "$@"
    local dependency
    for dependency in git docker python3 sha256sum readelf grep cut cp chmod mkdir rm id mktemp cat; do
        require_command "$dependency"
    done
    check_repository
    work_dir=$(mktemp -d "${TMPDIR:-/tmp}/kami-release.XXXXXXXXXX")
    trap cleanup EXIT
    update_versions
    build_release
    package_release
    commit_and_tag
    print_release_info
}

main "$@"
