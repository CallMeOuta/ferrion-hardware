#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-only
set -euo pipefail

usage="Brings a later revision of the upstream base repository's drivers into drivers/ by a
three-way merge.

usage: scripts/drivers-upstream.sh fetch
       scripts/drivers-upstream.sh check [<rev>]
       scripts/drivers-upstream.sh merge <rev>

fetch  fetches upstream's branches into refs/upstream/base/ of this clone; they are never pushed
check  merges, in memory, what upstream changed under drivers/ between the recorded revision and
       <rev> (default: upstream's default branch) into HEAD, prints each conflicting path and
       exits 1 when there is one
merge  does the same merge in the index and the working tree, writes <rev> into
       drivers/upstream.toml and leaves the result uncommitted

The record is drivers/upstream.toml: upstream's repository, the revision drivers/ holds, and the
folders of upstream's drivers/ that are not kept here. Upstream's files are compared with their
license line already in place, so that line never conflicts and a file upstream adds arrives
with it. FerrionOS merges the same revision into its components/base with its own
scripts/component-upstream.sh, and its pin moves to the commit made here.
Run from a clean checkout."

case "${1:-}" in
    -h|--help) echo "$usage"; exit 0 ;;
esac
[ $# -ge 1 ] || { echo "$usage" >&2; exit 2; }
action=$1 target=${2:-}
here=$(cd "$(dirname "$0")" && pwd)
cd "$(git rev-parse --show-toplevel)"
record=drivers/upstream.toml

read_record() {
    python3 -c 'import sys, tomllib
value = tomllib.load(open(sys.argv[1], "rb")).get(sys.argv[2], "")
print(value if isinstance(value, str) else "\n".join(value))' "$record" "$1"
}

url=$(read_record url)
old=$(read_record rev)
[ -n "$url" ] && [ -n "$old" ] || { echo "$record has no url and rev" >&2; exit 1; }

fetch_refs() {
    git fetch --quiet --no-tags "$url" "+refs/heads/*:refs/upstream/base/*" "+HEAD:refs/upstream/base/HEAD"
}

wrap() {
    local commit=$1 index tree
    index=$(mktemp -u)
    GIT_INDEX_FILE=$index git read-tree --prefix=drivers/ "$commit:drivers"
    read_record left-out | while read -r folder; do
        [ -z "$folder" ] || GIT_INDEX_FILE=$index git rm --quiet --cached -r --ignore-unmatch -- "$folder"
    done
    tree=$(GIT_INDEX_FILE=$index git write-tree)
    rm -f "$index"
    python3 "$here/license-check.py" mark-tree "$tree"
}

case "$action" in
    fetch)
        fetch_refs
        ;;
    check|merge)
        [ "$action" = check ] || [ -n "$target" ] || { echo "$usage" >&2; exit 2; }
        fetch_refs
        new=$(git rev-parse --verify --quiet "${target:-refs/upstream/base/HEAD}^{commit}") \
            || { echo "unknown revision ${target:-HEAD}" >&2; exit 1; }
        git merge-base --is-ancestor "$old" "$new" \
            || { echo "$new does not descend from the recorded $old" >&2; exit 1; }
        base=$(git commit-tree -m "upstream drivers $old" "$(wrap "$old")")
        theirs=$(git commit-tree -p "$base" -m "upstream drivers $new" "$(wrap "$new")")
        echo "drivers: $(git rev-list --count "$old..$new") upstream commits and" \
            "$(git diff --name-only "$base" "$theirs" | wc -l) changed files under drivers/ from $old to $new"
        [ "$(git rev-parse "$old:LICENSE")" = "$(git rev-parse "$new:LICENSE")" ] \
            || echo "drivers: upstream's LICENSE changed; read the change, then update drivers/LICENSE-MIT and its sha256 in $record"
        if [ "$action" = check ]; then
            if merged=$(git merge-tree --write-tree --name-only --no-messages --merge-base="$base" HEAD "$theirs"); then
                echo "drivers: merges without a conflict"
            else
                echo "$merged" | tail -n +2 | sed 's/^/conflict: /'
                exit 1
            fi
        else
            [ -z "$(git status --porcelain)" ] || { echo "commit or stash your changes first" >&2; exit 1; }
            conflicts=0
            git cherry-pick --no-commit --allow-empty "$theirs" || conflicts=1
            python3 -c 'import sys
path, old, new = sys.argv[1:]
text = open(path, encoding="utf-8").read()
assert text.count(old) == 1, f"{old} is not written exactly once in {path}"
open(path, "w", encoding="utf-8").write(text.replace(old, new))' "$record" "$old" "$new"
            git add -- "$record"
            if [ "$conflicts" = 1 ]; then
                echo "drivers: resolve the conflicts below, then git add each file:"
                git diff --name-only --diff-filter=U
            fi
            echo "drivers: next, python3 scripts/license-check.py check HEAD $theirs, the checks of" \
                "CONTRIBUTING.md against FerrionOS with the same revision merged, and one commit that names $old and $new"
        fi
        ;;
    *)
        echo "$usage" >&2
        exit 2
        ;;
esac
