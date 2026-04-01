#!/usr/bin/env bash
# generate-sitemap.sh — Creates sitemap.xml from the assembled _site directory.
# Usage: ./docs/scripts/generate-sitemap.sh <site-dir> <base-url>
#   e.g. ./docs/scripts/generate-sitemap.sh _site https://torvyn.github.io/torvyn

set -euo pipefail

SITE_DIR="${1:?Usage: generate-sitemap.sh <site-dir> <base-url>}"
BASE_URL="${2:?Usage: generate-sitemap.sh <site-dir> <base-url>}"

# Strip trailing slash from base URL
BASE_URL="${BASE_URL%/}"

OUTFILE="${SITE_DIR}/sitemap.xml"

cat > "$OUTFILE" <<'HEADER'
<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
HEADER

# Find all .html files, exclude print.html and 404.html
find "$SITE_DIR" -name '*.html' -not -name 'print.html' -not -name '404.html' | sort | while read -r file; do
    # Convert file path to URL path
    path="${file#"$SITE_DIR"}"
    # Strip /index.html to get clean URL
    path="${path%/index.html}"
    [ -z "$path" ] && path="/"

    # Priority: landing page highest, docs next, api lower
    priority="0.5"
    if [ "$path" = "/" ]; then
        priority="1.0"
    elif echo "$path" | grep -q "^/docs/getting-started"; then
        priority="0.8"
    elif echo "$path" | grep -q "^/docs/concepts"; then
        priority="0.7"
    elif echo "$path" | grep -q "^/docs"; then
        priority="0.6"
    fi

    cat >> "$OUTFILE" <<EOF
  <url>
    <loc>${BASE_URL}${path}</loc>
    <priority>${priority}</priority>
  </url>
EOF
done

cat >> "$OUTFILE" <<'FOOTER'
</urlset>
FOOTER

echo "Generated sitemap at ${OUTFILE} with $(grep -c '<url>' "$OUTFILE") URLs"
