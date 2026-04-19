#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pkgbuild_path="$repo_root/packaging/aur/PKGBUILD"
srcinfo_path="$repo_root/packaging/aur/.SRCINFO"

usage() {
  cat <<'EOF'
Usage:
  scripts/bump-codex-frontend.sh --latest
  scripts/bump-codex-frontend.sh <version> <zip_path>

Examples:
  scripts/bump-codex-frontend.sh --latest
  scripts/bump-codex-frontend.sh 26.415.40636 /tmp/Codex-darwin-arm64-26.415.40636.zip
EOF
}

if [[ ! -f "$pkgbuild_path" ]]; then
  echo "missing PKGBUILD at $pkgbuild_path" >&2
  exit 1
fi

latest_from_appcast() {
  "$repo_root/scripts/check-codex-upstream.sh" --plain
}

if [[ "${1:-}" == "--latest" ]]; then
  appcast_metadata="$(latest_from_appcast)"
  version="$(printf '%s\n' "$appcast_metadata" | awk -F= '/^version=/{print $2}')"
  zip_url="$(printf '%s\n' "$appcast_metadata" | awk -F= '/^zip_url=/{print $2}')"
  temp_dir="$(mktemp -d)"
  trap 'rm -rf "$temp_dir"' EXIT
  zip_path="$temp_dir/Codex-darwin-arm64-${version}.zip"
  curl -fLo "$zip_path" "$zip_url"
elif [[ $# -eq 2 ]]; then
  version="$1"
  zip_path="$2"
else
  usage >&2
  exit 1
fi

if [[ ! -f "$zip_path" ]]; then
  echo "zip not found: $zip_path" >&2
  exit 1
fi

sha256="$(sha256sum "$zip_path" | awk '{print $1}')"
zip_file_name="Codex-darwin-arm64-${version}.zip"

perl -0pi -e "s/_codex_frontend_version=.*/_codex_frontend_version=${version}/" "$pkgbuild_path"
perl -0pi -e "s/Codex-darwin-arm64-[0-9.]+\\.zip/${zip_file_name}/g" "$pkgbuild_path"
perl -0pi -e "s/'[0-9a-f]{64}'/'${sha256}'/" "$pkgbuild_path"

if command -v makepkg >/dev/null 2>&1; then
  (
    cd "$repo_root/packaging/aur"
    makepkg --printsrcinfo > .SRCINFO
  )
else
  echo "warning: makepkg not found, skipping .SRCINFO refresh" >&2
fi

printf 'Updated frontend metadata\n'
printf '  Version: %s\n' "$version"
printf '  SHA256:  %s\n' "$sha256"
printf '  PKGBUILD: %s\n' "$pkgbuild_path"
if [[ -f "$srcinfo_path" ]]; then
  printf '  SRCINFO: %s\n' "$srcinfo_path"
fi
