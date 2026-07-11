# MKV Player — Nextcloud app

Plays Matroska `.mkv` / `.mka` files directly in Nextcloud's built-in **Viewer** — in-browser,
offline, with no server-side transcoding. The player itself (WASM MKV→fMP4 remux + MSE + video.js
controls, libass/PGS subtitles, optional ffmpeg.wasm audio transcoding, corner watermark) is the
`mkv-player-ui` library (`../player-lib`); this app wraps it in a Viewer handler, bundles a
royalty-free ffmpeg.wasm core served from the instance, and adds admin settings.

The app is **freemium**: free and fully functional, with a small corner watermark that an optional
paid license removes. See [Licensing & watermark](#licensing--watermark).

## Build

```sh
npm install
npm run deploy      # = build:app (Vite) + setup:ffmpeg (copy the core) + assemble (stage deploy/mkvplayer)
```

`npm run build` additionally rebuilds the Rust `mkv-player` WASM first; use it when that changed.

The Vite build uses `@nextcloud/vite-config` with two required overrides for this wasm/worker
library (see `vite.config.js`): `nodePolyfills:false` and a relative `renderBuiltUrl` (so the
jassub/ffmpeg workers don't reference `window.OC`, which doesn't exist in a Worker). It emits **two
JS entries** — `mkvplayer-main` (the Viewer handler) and `mkvplayer-admin-settings` (the license
settings Vue app). Output lands in the app root — `js/` (entries), `dist/` (wasm), `assets/`
(workers), `css/`.

## Local dev instance (podman)

```sh
npm run deploy
podman-compose up -d          # Nextcloud 34 + SQLite, auto-installs (~30s)
bash dev/sync.sh              # copy the staged app in + enable it
# → http://localhost:8080   (admin / admin)
```

Re-run `npm run deploy && bash dev/sync.sh` after **any** change, then **hard-reload** the browser.
Two gotchas that look like "my change didn't apply":

- `dev/sync.sh` copies from `deploy/mkvplayer/`, which only `npm run assemble` (part of `deploy`)
  refreshes. Running `build:app` alone updates the app root but not `deploy/`, so always run the
  full `deploy` before syncing.
- The app version is unchanged between builds, so Nextcloud's asset cache-buster (`?v=…`) stays the
  same and the browser serves cached JS/CSS. Hard-reload (Ctrl+Shift+R), or keep DevTools open with
  “Disable cache”.

The app is **not** bind-mounted: bind-mounting into `/var/www/html/custom_apps/…` makes rootless
podman pre-create that path owned by root, and NC's installer then aborts with "Cannot write into
apps directory" because the writable `custom_apps` path isn't writable by `www-data`. So we install
cleanly first and `podman cp` the app in (`dev/sync.sh`). Only the built assets + PHP are staged
(`scripts/assemble.sh`) — never `node_modules`.

Teardown: `podman-compose down` (add `-v` / `podman volume rm player-nextcloud_nextcloud_html` to
wipe the instance).

## Audio transcoding (offline)

Matroska video always plays via in-browser remuxing. Audio codecs the browser can't decode natively
in the remuxed MP4 (Vorbis, AC-3, E-AC-3, DTS core, …) are transcoded to **AAC-LC** (preferred —
universal, incl. Safari) or **Opus** with ffmpeg.wasm.

The app **bundles an audio-only ffmpeg.wasm core and serves it from your own server**: no external
requests, works offline. It is **LGPL / AGPL-compatible** (all native/BSD codecs, no x264/x265). It
decodes Vorbis/Opus/FLAC/ALAC/PCM plus AAC-LC, AC-3, E-AC-3 and DTS core, and encodes AAC-LC/Opus.
The lossy codecs are royalty-free or have expired core patents (AAC-LC, AC-3, E-AC-3, DTS core).
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
core/wasm URLs. ⚠️ That external option makes each viewer's browser contact the third-party host
(privacy: exposes their IP); it's off by default. The app adds the configured origin to the CSP
`connect-src` only when it's enabled.

> **Video is remuxed, not transcoded.** The video track must be one the browser can decode: H.264
> everywhere; HEVC/H.265, VP9, AV1 only where the browser/OS supports them. Unsupported video won't
> play — the app cannot convert it.

## Licensing & watermark

The player shows a small watermark in the bottom-right corner of the video. It stays visible but
fades and lifts above the control bar with the player UI (pure CSS off the skin's control-visibility
state; no per-frame JS). A **valid license hides the watermark**.

**Admin settings** (*Administration → MKV Player*) add, alongside the ffmpeg options, a custom Vue
section with a masked **License key** field, a **Save & validate** action that shows the result, and
a **Buy** link that opens the purchase page with this instance's id appended.

**How validation works (fully offline):**

- A license key is `base64url(payload) . "." . base64url(ed25519_signature(payload))`, where
  `payload` is url-encoded `email=<addr>&nc_instance=<instanceid>`.
- The server verifies the Ed25519 signature against a built-in public key and requires the payload's
  `nc_instance` to equal *this* instance's `instanceid` — so a key is bound to one instance.
- The raw key **never reaches the frontend**; only a `licensed` boolean is passed to the Viewer (via
  initial state). Nothing is sent to any external server for validation, and no usage data leaves
  the instance.

Implemented in `lib/Service/LicenseService.php` (validation + buy URL), `lib/Settings/
LicenseAdminSettings.php` + `src/AdminSettings.vue` (admin UI), and `lib/Controller/
LicenseController.php` (`POST /settings/license`, admin-only). The watermark itself is a
forced by `mkv-player-ui` unless it's told the session is licensed; `src/views/player-view.js`
passes `embedderValidatedLicense` (a trusted vouch, not the key) only on licensed instances.

**Before shipping, replace the placeholder public key** in `lib/Service/LicenseService.php`:

- `PUBLIC_KEY_HEX` — swap the generated **test** public key for your production one.

(`BUY_URL` is already set to the live landing page, `https://matroska.davidschneider.xyz/nextcloud`;
`getBuyUrl()` substitutes `%NC%` with the instance id.)

**Minting test keys** (dev only). The test keypair lives in `dev/license-test-keys.txt`
(gitignored); `scripts/sign-license.php` signs a key with the private seed:

```sh
# instance id: occ config:system:get instanceid
#   podman exec -u www-data mkvplayer-nc php occ config:system:get instanceid
MKV_LICENSE_SEED_HEX=<seed-from-dev/license-test-keys.txt> \
  php scripts/sign-license.php test@example.com <instanceid>
```

Paste the printed key into the admin License key field; the watermark disappears once it validates.

## File types

`.mkv` (`video/x-matroska`) works out of the box. `.mka` (Matroska audio) is **not** enabled yet:
Nextcloud has no default MIME mapping for `.mka` (it detects as `application/octet-stream`) and the
Viewer matches handlers by MIME only, so an admin would need to map `.mka` → `audio/x-matroska`
(custom `config/mimetypemapping.json` + `occ maintenance:mimetype:update-db`). The handler already
registers `audio/x-matroska`, so it will work once that mapping exists — but this is a video player,
so audio-only playback is just a byproduct.
