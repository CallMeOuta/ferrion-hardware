#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-only
set -eu

out=$1
nm=$2
refusals=$3
script=$4
shim_objs=$5
libs=$6
mode=${7:-census}

defined() {
    $nm --defined-only -g "$@" 2>/dev/null | awk 'NF == 3 && $2 != "U" { print $3 }' | sort -u
}

$nm -u "$out/linux.o" | awk '{ print $2 }' | sort -u > "$out/needed.txt"
defined $libs > "$out/libs-defined.txt"
{
    defined $shim_objs
    sed -n 's/^[[:space:]]*\([A-Za-z_][A-Za-z0-9_]*\)[[:space:]]*=.*;$/\1/p' "$script"
} | sort -u > "$out/shim-defined.txt"
awk -F'\t' '$1 !~ /^#/ && $1 != "symbol" && NF == 3 { print $1 }' "$refusals" | sort -u > "$out/refusal-names.txt"
$nm -u $shim_objs | awk 'NF == 2 && $1 == "U" { print $2 }' | sort -u > "$out/shim-needed.txt"
sort -u "$out/needed.txt" "$out/shim-needed.txt" > "$out/closure-needed.txt"

comm -12 "$out/needed.txt" "$out/libs-defined.txt" > "$out/libc.txt"
comm -12 "$out/needed.txt" "$out/shim-defined.txt" > "$out/implemented.txt"
comm -12 "$out/needed.txt" "$out/refusal-names.txt" > "$out/refused.txt"
comm -23 "$out/refusal-names.txt" "$out/closure-needed.txt" > "$out/refused-unneeded.txt"
comm -23 "$out/refusal-names.txt" "$out/needed.txt" | comm -12 - "$out/shim-needed.txt" > "$out/refused-support.txt"
$nm -u "$out/host.o" | awk '{ print $2 }' | sort -u | comm -23 - "$out/libs-defined.txt" | comm -23 - "$out/shim-defined.txt" \
    | grep -vx '_GLOBAL_OFFSET_TABLE_' > "$out/unresolved.txt" || true

{
    echo "needed $(wc -l < "$out/needed.txt")"
    echo "libc $(wc -l < "$out/libc.txt")"
    echo "implemented $(wc -l < "$out/implemented.txt")"
    echo "refused $(wc -l < "$out/refused.txt")"
    echo "support_refused $(wc -l < "$out/refused-support.txt")"
    echo "unresolved $(wc -l < "$out/unresolved.txt")"
} > "$out/summary.txt"

if [ "$mode" = check ]; then
    cat "$out/summary.txt"
    status=0
    if [ -s "$out/refused-unneeded.txt" ]; then
        echo "refusal table names symbols nothing needs:"
        sed 's/^/  /' "$out/refused-unneeded.txt"
        status=1
    fi
    if [ -s "$out/unresolved.txt" ]; then
        echo "unresolved symbols:"
        sed 's/^/  /' "$out/unresolved.txt"
        status=1
    fi
    exit $status
fi
