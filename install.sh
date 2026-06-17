#!/bin/sh
# lore installer.  builds lore from source with cargo and installs it.
#
#   curl -fsSL https://lorevcs.com/install.sh | sh
#
set -eu

repo="https://github.com/lorevcs/lore"

note() { printf 'lore: %s\n' "$*" >&2; }

if ! command -v cargo >/dev/null 2>&1; then
	note "cargo not found.  install rust from https://rustup.rs and run this again."
	exit 1
fi

note "building and installing from ${repo}"
cargo install --git "$repo" --locked lore

bindir="${CARGO_HOME:-$HOME/.cargo}/bin"
note "installed.  make sure ${bindir} is on your PATH, then run: lore"
