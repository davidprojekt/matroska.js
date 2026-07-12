#!/usr/bin/env bash
# Push the staged app (deploy/matroskaplayer) into the running dev container and enable it. Run after
# `npm run deploy` and any code change. Copies into the www-data-owned custom_apps rather than
# bind-mounting (see compose.yml for why), so it works with rootless podman.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
c=matroskaplayer-nc
occ() { podman exec -u www-data "$c" php occ "$@"; }

[ -d "$here/deploy/matroskaplayer" ] || { echo "deploy/matroskaplayer missing — run 'npm run deploy' first"; exit 1; }

# Wait until the instance is installed (first boot takes ~30s).
for i in $(seq 1 30); do
  if podman exec -u www-data "$c" php occ status 2>/dev/null | grep -q "installed: true"; then break; fi
  [ "$i" = 30 ] && { echo "Nextcloud not installed yet — check 'podman logs $c'"; exit 1; }
  sleep 3
done

# Replace the app dir in the container, then enable + refresh.
podman exec "$c" rm -rf /var/www/html/custom_apps/matroskaplayer
podman cp "$here/deploy/matroskaplayer" "$c":/var/www/html/custom_apps/matroskaplayer
podman exec "$c" chown -R www-data:www-data /var/www/html/custom_apps/matroskaplayer
occ app:enable matroskaplayer 2>/dev/null || occ app:update matroskaplayer || true
occ maintenance:mimetype:update-db >/dev/null 2>&1 || true
echo "synced + enabled matroskaplayer → http://localhost:8080"
