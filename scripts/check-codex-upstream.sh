#!/usr/bin/env bash

set -euo pipefail

appcast_url="${CODEX_APPCAST_URL:-https://persistent.oaistatic.com/codex-app-prod/appcast.xml}"

appcast_xml="$(curl -fsSL "$appcast_url")"
latest_item_xml="$(printf '%s\n' "$appcast_xml" | sed -n '/<item>/,/<\/item>/p' | sed -n '1,/<\/item>/p')"

latest_version="$(printf '%s\n' "$latest_item_xml" | grep -m1 -oP '(?<=<title>)[^<]+')"
latest_pub_date="$(printf '%s\n' "$latest_item_xml" | grep -m1 -oP '(?<=<pubDate>)[^<]+')"
latest_zip_url="$(printf '%s\n' "$latest_item_xml" | grep -m1 -oP '(?<=<enclosure url=")[^"]+')"

if [[ -z "$latest_version" || -z "$latest_pub_date" || -z "$latest_zip_url" ]]; then
  echo "failed to parse latest Codex release from $appcast_url" >&2
  exit 1
fi

if [[ "${1:-}" == "--plain" ]]; then
  printf 'version=%s\n' "$latest_version"
  printf 'pub_date=%s\n' "$latest_pub_date"
  printf 'zip_url=%s\n' "$latest_zip_url"
  exit 0
fi

printf 'Latest Codex macOS frontend release\n'
printf '  Version:  %s\n' "$latest_version"
printf '  Published: %s\n' "$latest_pub_date"
printf '  Bundle:   %s\n' "$latest_zip_url"
