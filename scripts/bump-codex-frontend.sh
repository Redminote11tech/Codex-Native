#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pkg_dirs=("$repo_root/packaging/aur" "$repo_root/packaging/aur-bin")
is_root="${EUID:-$(id -u)}"

usage() {
  cat <<'EOF'
Usage:
  scripts/bump-codex-frontend.sh --latest
  scripts/bump-codex-frontend.sh <version> <zip_path>

Examples:
  scripts/bump-codex-frontend.sh --latest
  scripts/bump-codex-frontend.sh 26.818.41705 /tmp/ChatGPT-darwin-arm64-26.818.41705.zip
EOF
}

refresh_srcinfo_without_makepkg() {
  local dir="$1"
  local srcinfo_path="$dir/.SRCINFO"
  if [[ ! -f "$srcinfo_path" ]]; then
    echo "warning: missing .SRCINFO template in $dir, skipping refresh" >&2
    return 0
  fi

  perl -0pi -e "s/(pkgver = ).*/\${1}r0.0/" "$srcinfo_path"
  perl -0pi -e "s/(Codex|ChatGPT)-darwin-arm64-[0-9.]+\\.zip/${zip_file_name}/g" "$srcinfo_path"
  perl -0pi -e "s/_codex_frontend_version = .*/_codex_frontend_version = ${version}/" "$srcinfo_path"
  perl -0pi -e "s/_codex_frontend_sha256 = .*/_codex_frontend_sha256 = ${sha256}/" "$srcinfo_path"
}

latest_from_appcast() {
  "$repo_root/scripts/check-codex-upstream.sh" --plain
}

if [[ "${1:-}" == "--latest" ]]; then
  appcast_metadata="$(latest_from_appcast)"
  version="$(printf '%s\n' "$appcast_metadata" | awk -F= '/^version=/{print $2}')"
  zip_url="$(printf '%s\n' "$appcast_metadata" | awk -F= '/^zip_url=/{print $2}')"
  zip_file_name="$(basename "$zip_url")"
  temp_dir="$(mktemp -d)"
  trap 'rm -rf "$temp_dir"' EXIT
  zip_path="$temp_dir/$zip_file_name"
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
zip_file_name="$(basename "$zip_path")"
artifact_prefix="${zip_file_name%-${version}.zip}"

for pkg_dir in "${pkg_dirs[@]}"; do
  pkgbuild_path="$pkg_dir/PKGBUILD"
  if [[ ! -f "$pkgbuild_path" ]]; then
    echo "missing PKGBUILD at $pkgbuild_path" >&2
    exit 1
  fi

  perl -0pi -e "s/_codex_frontend_version=.*/_codex_frontend_version=${version}/" "$pkgbuild_path"
  perl -0pi -e "s/_codex_frontend_artifact=.*/_codex_frontend_artifact=${artifact_prefix}/" "$pkgbuild_path"
  perl -0pi -e "s/(Codex|ChatGPT)-darwin-arm64-[0-9.]+\\.zip/${zip_file_name}/g" "$pkgbuild_path"
  perl -0pi -e "s/_codex_frontend_sha256='[^']*'/_codex_frontend_sha256='${sha256}'/" "$pkgbuild_path"

  if command -v makepkg >/dev/null 2>&1 && [[ "$is_root" -ne 0 ]]; then
    (
      cd "$pkg_dir"
      makepkg --printsrcinfo > .SRCINFO
    )
  else
    if [[ "$is_root" -eq 0 ]]; then
      echo "warning: running as root, using fallback .SRCINFO refresh" >&2
    else
      echo "warning: makepkg not found, using fallback .SRCINFO refresh" >&2
    fi
    refresh_srcinfo_without_makepkg "$pkg_dir"
  fi
done

printf 'Updated frontend metadata\n'
printf '  Version: %s\n' "$version"
printf '  SHA256:  %s\n' "$sha256"
printf '  Packages: %s\n' "${pkg_dirs[*]}"
