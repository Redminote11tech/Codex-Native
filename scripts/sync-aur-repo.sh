#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="$repo_root/packaging/aur"
target_dir="${1:-}"
pkgver="r$(git -C "$repo_root" rev-list --count HEAD).$(git -C "$repo_root" rev-parse --short HEAD)"
is_root="${EUID:-$(id -u)}"

if [[ -z "$target_dir" ]]; then
  echo "usage: scripts/sync-aur-repo.sh /path/to/aur-repo" >&2
  exit 1
fi

if [[ ! -d "$target_dir/.git" ]]; then
  echo "target is not a git repository: $target_dir" >&2
  exit 1
fi

install -Dm644 "$source_dir/PKGBUILD" "$target_dir/PKGBUILD"
install -Dm644 "$source_dir/codex-native.desktop" "$target_dir/codex-native.desktop"
install -Dm755 "$source_dir/codex-native-launcher" "$target_dir/codex-native-launcher"

if [[ -f "$source_dir/README.md" ]]; then
  install -Dm644 "$source_dir/README.md" "$target_dir/README.md"
fi

sed -i "s/^pkgver=.*/pkgver=$pkgver/" "$target_dir/PKGBUILD"

if command -v makepkg >/dev/null 2>&1 && [[ "$is_root" -ne 0 ]]; then
  (
    cd "$target_dir"
    makepkg --printsrcinfo > .SRCINFO
  )
else
  if [[ "$is_root" -eq 0 ]]; then
    echo "warning: running as root, using fallback .SRCINFO refresh" >&2
  fi
  install -Dm644 "$source_dir/.SRCINFO" "$target_dir/.SRCINFO"
  sed -i "s/^\(\s*pkgver = \).*/\1$pkgver/" "$target_dir/.SRCINFO"
fi
