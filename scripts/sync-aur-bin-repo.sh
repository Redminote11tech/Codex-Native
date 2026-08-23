#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="$repo_root/packaging/aur-bin"
target_dir="${1:-}"
release_tag="${RELEASE_TAG:-}"
release_asset_sha256="${RELEASE_ASSET_SHA256:-}"
pkgver="${RELEASE_TAG:-}"
is_root="${EUID:-$(id -u)}"

if [[ -z "$target_dir" ]]; then
  echo "usage: scripts/sync-aur-bin-repo.sh /path/to/aur-repo" >&2
  exit 1
fi

if [[ ! -d "$target_dir/.git" ]]; then
  echo "target is not a git repository: $target_dir" >&2
  exit 1
fi

if [[ -z "$release_tag" || -z "$release_asset_sha256" ]]; then
  echo "RELEASE_TAG and RELEASE_ASSET_SHA256 must be set" >&2
  exit 1
fi

if [[ ! "$release_asset_sha256" =~ ^[0-9a-f]{64}$ ]]; then
  echo "RELEASE_ASSET_SHA256 must be a sha256 hex digest, got: $release_asset_sha256" >&2
  exit 1
fi

if [[ ! "$pkgver" =~ ^r[0-9]+\.[0-9a-f]+$ ]]; then
  echo "RELEASE_TAG must match the release naming scheme r<count>.<short-sha>, got: $pkgver" >&2
  exit 1
fi

install -Dm644 "$source_dir/PKGBUILD" "$target_dir/PKGBUILD"
install -Dm755 "$repo_root/packaging/aur/codex-native-launcher" "$target_dir/codex-native-launcher"
install -Dm644 "$repo_root/packaging/aur/codex-native.desktop" "$target_dir/codex-native.desktop"

if [[ -f "$repo_root/packaging/aur/README.md" ]]; then
  install -Dm644 "$repo_root/packaging/aur/README.md" "$target_dir/README.md"
fi

sed -i "s/^pkgver=.*/pkgver=$pkgver/" "$target_dir/PKGBUILD"
sed -i "s/^_release_tag=.*/_release_tag=$release_tag/" "$target_dir/PKGBUILD"
sed -i "s/^_release_asset_sha256=.*/_release_asset_sha256='$release_asset_sha256'/" "$target_dir/PKGBUILD"

patch_srcinfo() {
  local file="$1"
  sed -i \
    -e "s/^\(\s*pkgver = \).*/\1$pkgver/" \
    -e "s/^\(\s*_release_tag = \).*/\1$release_tag/" \
    -e "s/^\(\s*_release_asset_sha256 = \).*/\1$release_asset_sha256/" \
    "$file"
  # Rewrite expanded values: source URLs embed the release tag, and the asset
  # checksum sits on the sha256sums line directly below the tarball source.
  sed -i -E "/linux-x86_64\.tar\.gz/s/r[0-9]+\.[0-9a-f]+/$release_tag/g" "$file"
  # Asset checksum is the first sha256sums line (array order); source entries
  # are not adjacent to their checksums in .SRCINFO.
  perl -0pi -e 's{^([ \t]*sha256sums = )(?:[0-9a-f]{64}|SKIP)}{$1'"$release_asset_sha256"'}m' "$file"
}
if command -v makepkg >/dev/null 2>&1 && [[ "$is_root" -ne 0 ]]; then
  (
    cd "$target_dir"
    makepkg --printsrcinfo > .SRCINFO
  )
else
  if [[ "$is_root" -eq 0 ]]; then
    echo "warning: running as root, using fallback .SRCINFO refresh" >&2
  else
    echo "warning: makepkg not found, using fallback .SRCINFO refresh" >&2
  fi
  install -Dm644 "$source_dir/.SRCINFO" "$target_dir/.SRCINFO"
  patch_srcinfo "$target_dir/.SRCINFO"
fi
