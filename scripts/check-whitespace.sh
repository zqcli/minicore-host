#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Compare all committed development with the original project baseline. A
# shallow checkout would mistake its visible boundary for that baseline.
if [[ "$(git rev-parse --is-shallow-repository)" != false ]]; then
    printf '%s\n' 'Whitespace checks require full Git history; fetch with depth 0.' >&2
    exit 1
fi
mapfile -t roots < <(git rev-list --max-parents=0 HEAD)
if [[ ${#roots[@]} != 1 ]]; then
    printf '%s\n' 'Whitespace checks require one unambiguous project root commit.' >&2
    exit 1
fi
git diff --check "${roots[0]}" HEAD
git diff --check --cached
git diff --check
