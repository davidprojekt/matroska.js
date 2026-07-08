# MKV Player — Nextcloud app

Plays Matroska `.mkv` / `.mka` files directly in Nextcloud's built-in **Viewer**. The player
itself (WASM MKV→fMP4 remux + MSE + video.js controls, libass subtitles, optional ffmpeg.wasm
audio transcoding) is the `mkv-player-ui` library (`../player-lib`); this app wraps it in a Viewer
handler and adds admin settings for the ffmpeg core URLs.

## Build

```sh
npm install
npm run deploy      # = build:app (Vite) + assemble (stage deploy/mkvplayer)
```

`npm run build` additionally rebuilds the Rust `mkv-player` WASM first; use it when that changed.

The Vite build uses `@nextcloud/vite-config` with two required overrides for this wasm/worker
library (see `vite.config.js`): `nodePolyfills:false` and a relative `renderBuiltUrl` (so the
jassub/ffmpeg workers don't reference `window.OC`, which doesn't exist in a Worker). Output lands
in the app root — `js/` (entries), `dist/` (wasm), `assets/` (workers), `css/`.

## Local dev instance (podman)

```sh
npm run deploy
podman-compose up -d          # Nextcloud 30 + SQLite, auto-installs (~30s)
bash dev/sync.sh              # copy the staged app in + enable it
# → http://localhost:8080   (admin / admin)
```

Re-run `npm run deploy && bash dev/sync.sh` after code changes, then hard-reload the browser.

The app is **not** bind-mounted: bind-mounting into `/var/www/html/custom_apps/…` makes rootless
podman pre-create that path owned by root, and NC's installer then aborts with "Cannot write into
apps directory" because the writable `custom_apps` path isn't writable by `www-data`. So we
install cleanly first and `podman cp` the app in (`dev/sync.sh`). Only the built assets + PHP are
staged (`scripts/assemble.sh`) — never `node_modules`.

Teardown: `podman-compose down` (add `-v` / `podman volume rm player-nextcloud_nextcloud_html` to
wipe the instance).

## Audio transcoding (admin settings)

Matroska video always plays via in-browser remuxing. Audio codecs the browser can't decode
natively (AC-3, DTS, …) can optionally be transcoded with ffmpeg.wasm — but **no ffmpeg core ships
with the app**. Under *Administration settings → MKV Player*, enable transcoding and provide the
URLs of an ffmpeg.wasm build (single-thread ESM `ffmpeg-core.js` + `ffmpeg-core.wasm`). The URLs
must be reachable by the browser (same-origin, or a host sending permissive CORS — the app adds the
configured origin to the CSP `connect-src` automatically). Until configured, transcoding stays off.

## File types

`.mkv` (`video/x-matroska`) works out of the box. `.mka` (Matroska audio) is **not** enabled yet:
Nextcloud has no default MIME mapping for `.mka` (it detects as `application/octet-stream`) and the
Viewer matches handlers by MIME only, so an admin would need to map `.mka` → `audio/x-matroska`
(custom `config/mimetypemapping.json` + `occ maintenance:mimetype:update-db`). The handler already
registers `audio/x-matroska`, so it will work once that mapping exists.
