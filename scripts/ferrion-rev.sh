#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-only
set -euo pipefail

usage="usage: scripts/ferrion-rev.sh <FerrionOS revision> [<FerrionOS checkout>]
The drivers take a few crates from FerrionOS's repository, named in drivers/Cargo.toml as git
dependencies at one revision. That revision is what a build of this repository alone uses;
FerrionOS's own build replaces those crates with its current tree and never reads it.
Sets the revision to <FerrionOS revision>, a full commit hash, and resolves drivers/Cargo.lock
again. With <FerrionOS checkout>, cargo reads the revision from that local clone instead of from
the network, for a commit of FerrionOS that is not pushed yet."
[ $# -ge 1 ] && [ $# -le 2 ] && [[ $1 =~ ^[0-9a-f]{40}$ ]] || { echo "$usage" >&2; exit 2; }
rev=$1
cd "$(git rev-parse --show-toplevel)/drivers"
url=$(sed -n 's/^[a-z-]* = { git = "\(https:[^"]*FerrionOS[^"]*\)", rev = "[0-9a-f]\{40\}" }$/\1/p' Cargo.toml | sort -u)
[ -n "$url" ] && [ "$(echo "$url" | wc -l)" -eq 1 ] || { echo "drivers/Cargo.toml names no single FerrionOS repository" >&2; exit 1; }
sed -i "s#\(git = \"$url\", rev = \"\)[0-9a-f]\{40\}\"#\1$rev\"#" Cargo.toml
if [ $# -eq 2 ]; then
    local_clone=$(cd "$2" && git rev-parse --show-toplevel)
    git -C "$local_clone" cat-file -e "$rev^{commit}" || { echo "$2 has no commit $rev" >&2; exit 1; }
    export CARGO_NET_GIT_FETCH_WITH_CLI=true GIT_CONFIG_COUNT=1
    export GIT_CONFIG_KEY_0="url.file://$local_clone.insteadOf" GIT_CONFIG_VALUE_0="$url"
fi
cargo metadata --format-version 1 > /dev/null
echo "drivers/Cargo.toml and Cargo.lock: $(grep -c "git = \"$url\"" Cargo.toml) crates of FerrionOS at $rev"
