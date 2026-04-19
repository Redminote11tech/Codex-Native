#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="$repo_root/packaging/aur"
target_dir="${1:-}"

if [[ -z "$target_dir" ]]; then
  echo "usage: scripts/sync-aur-repo.sh /path/to/aur-repo" >&2
  exit 1
fi

if [[ ! -d "$target_dir/.git" ]]; then
  echo "target is not a git repository: $target_dir" >&2
  exit 1
fi

install -Dm644 "$source_dir/PKGBUILD" "$target_dir/PKGBUILD"
install -Dm644 "$source_dir/.SRCINFO" "$target_dir/.SRCINFO"
install -Dm644 "$source_dir/codex-native.desktop" "$target_dir/codex-native.desktop"
install -Dm755 "$source_dir/codex-native-launcher" "$target_dir/codex-native-launcher"

if [[ -f "$source_dir/README.md" ]]; then
  install -Dm644 "$source_dir/README.md" "$target_dir/README.md"
fi
