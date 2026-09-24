#!/bin/sh
set -eu

readonly SCRIPT_NAME="${0##*/}"

usage() {
	printf 'Usage: %s [--help]\n' "$SCRIPT_NAME"
	printf 'Build kami locally in release mode. Prefers the hako Docker wrapper.\n'
}

die() {
	printf 'Error: %s\n' "$*" >&2
	exit 1
}

main() {
	case "${1:-}" in
	--help | -h)
		[ "$#" -eq 1 ] || die 'too many arguments'
		usage
		return 0
		;;
	'')
		[ "$#" -eq 0 ] || die 'unexpected empty argument'
		;;
	*)
		die "unknown argument: $1"
		;;
	esac

	command -v dirname >/dev/null 2>&1 || die 'required command not found: dirname'
	repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
	cd -- "$repo_root"

	if command -v docker >/dev/null 2>&1 && [ -x ./hako ]; then
		./hako cargo build --release --locked
	elif command -v cargo >/dev/null 2>&1; then
		cargo build --release --locked
	else
		die 'Docker/hako and Cargo are unavailable'
	fi

	printf 'Built %s/target/release/kami\n' "$repo_root"
}

main "$@"
