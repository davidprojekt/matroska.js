#!/usr/bin/env bash
# Push the staged app (deploy/mkvplayer) into the running dev container and enable it. Run after
# `npm run deploy` and any code change. Copies into the www-data-owned custom_apps rather than
# bind-mounting (see compose.yml for why), so it works with rootless podman.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
c=mkvplayer-nc
occ() { podman exec -u www-data "$c" php occ "$@"; }

[ -d "$here/deploy/mkvplayer" ] || { echo "deploy/mkvplayer missing — run 'npm run deploy' first"; exit 1; }

# Wait until the instance is installed (first boot takes ~30s).
for i in $(seq 1 30); do
  if podman exec -u www-data "$c" php occ status 2>/dev/null | grep -q "installed: true"; then break; fi
  [ "$i" = 30 ] && { echo "Nextcloud not installed yet — check 'podman logs $c'"; exit 1; }
  sleep 3
done

# Replace the app dir in the container, then enable + refresh.
podman exec "$c" rm -rf /var/www/html/custom_apps/mkvplayer
podman cp "$here/deploy/mkvplayer" "$c":/var/www/html/custom_apps/mkvplayer
podman exec "$c" chown -R www-data:www-data /var/www/html/custom_apps/mkvplayer
occ app:enable mkvplayer 2>/dev/null || occ app:update mkvplayer || true
occ maintenance:mimetype:update-db >/dev/null 2>&1 || true
echo "synced + enabled mkvplayer → http://localhost:8080"
