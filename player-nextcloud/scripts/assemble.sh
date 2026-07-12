#!/usr/bin/env bash
# Stage the *deployable* app into deploy/matroskaplayer/ — only what Nextcloud actually serves and
# runs: appinfo, PHP (lib), built assets (js/css/dist/assets), and static dirs (img/templates/
# l10n). Build-time artefacts (node_modules with its dangling workspace symlinks, src/, the Vite
# config, this repo's dev files) are deliberately excluded: shipping node_modules into the
# container makes the image entrypoint's `chown -R /var/www/html` crawl thousands of files and
# race the installer. The staged dir is also exactly what a release tarball would contain.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
out="$here/deploy/matroskaplayer"
rm -rf "$out"
mkdir -p "$out"

# Copy each deployable path that exists (lib/img/templates/l10n appear in later phases).
for p in appinfo lib js css dist assets ffmpeg img templates l10n composer.json; do
  [ -e "$here/$p" ] && cp -a "$here/$p" "$out/"
done
echo "[assemble] staged → deploy/matroskaplayer/ ($(find "$out" -type f | wc -l) files)"
