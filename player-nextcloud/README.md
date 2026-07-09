# MKV Player — Nextcloud app

Plays Matroska `.mkv` / `.mka` files directly in Nextcloud's built-in **Viewer**. The player
itself (WASM MKV→fMP4 remux + MSE + video.js controls, libass subtitles, optional ffmpeg.wasm
audio transcoding) is the `mkv-player-ui` library (`../player-lib`); this app wraps it in a Viewer
handler, bundles a royalty-free ffmpeg.wasm core served from the instance, and adds admin settings.

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

## Audio transcoding (offline)

Matroska video always plays via in-browser remuxing. Audio codecs the browser can't decode natively
in the remuxed MP4 (Vorbis, AC-3, E-AC-3, DTS core, …) are transcoded to **AAC-LC** (preferred —
universal, incl. Safari) or **Opus** with ffmpeg.wasm.

The app **bundles an audio-only ffmpeg.wasm core and serves it from your own server**: no external
requests, works offline. It is **LGPL / AGPL-compatible** (all native/BSD codecs, no x264/x265). It
decodes Vorbis/Opus/FLAC/ALAC/PCM plus AAC-LC, AC-3, E-AC-3 and DTS core, and encodes AAC-LC/Opus.
The lossy codecs are royalty-free or have expired core patents (AAC-LC, AC-3, DTS core) — **except
E-AC-3, which is newer and may still be patented in your jurisdiction; verify before distributing.**
Transcoding is on by default; the bundled `ffmpeg/LICENSE` and `ffmpeg/SOURCE.md` (the LGPL source
offer) ship alongside the core.

The core is **not committed** — it's compiled from source (see `../player-lib/ffmpeg-core/`) and
copied in at deploy time. Build order:

```sh
(cd ../player-lib && npm run build:ffmpeg)   # compile the core (Docker; slow once)
npm run deploy                                # build:app → setup:ffmpeg (copies the core) → assemble
```

If the core hasn't been built, `deploy` still works but transcoding is disabled until it exists.

**Still-patent-encumbered lossless codecs (TrueHD/MLP, DTS-HD) and HE-AAC** are excluded. To support
them, build a fuller ffmpeg.wasm yourself (`FFMPEG_PROFILE=full` — see
`player-lib/ffmpeg-core/README.md`), accepting the patent obligations, and point the app at it:
*Administration → MKV Player → **Advanced: load ffmpeg.wasm from an external server*** + the
core/wasm URLs. ⚠️ That external option
makes each viewer's browser contact the third-party host (privacy: exposes their IP); it's off by
default. The app adds the configured origin to the CSP `connect-src` only when it's enabled.

## File types

`.mkv` (`video/x-matroska`) works out of the box. `.mka` (Matroska audio) is **not** enabled yet:
Nextcloud has no default MIME mapping for `.mka` (it detects as `application/octet-stream`) and the
Viewer matches handlers by MIME only, so an admin would need to map `.mka` → `audio/x-matroska`
(custom `config/mimetypemapping.json` + `occ maintenance:mimetype:update-db`). The handler already
registers `audio/x-matroska`, so it will work once that mapping exists.
