# mkv-player-ui

The reusable browser MKV player extracted from `player-web`. It parses
`.mkv`/`.webm` with the `mkv-player` WASM remuxer, remuxes to fragmented MP4 on the fly,
and plays it through Media Source Extensions behind a [video.js v10](https://www.npmjs.com/package/@videojs/html)
control bar. Unsupported audio codecs are transcoded in-browser with ffmpeg.wasm; ASS/SSA
subtitles render via libass (JASSUB).

## Usage

```js
import { createPlayer } from 'mkv-player-ui';
import 'mkv-player-ui/style.css';

const player = createPlayer(document.querySelector('#player'), {
  controls: 'full',                       // 'full' | 'minimal' | 'none' | { preset, ...overrides }
  transcode: 'auto',                      // 'auto' | true | false
  ffmpeg: {                               // where to load the ffmpeg.wasm core from
    coreURL: 'https://cdn.example.com/ffmpeg/ffmpeg-core.js',
    wasmURL: 'https://cdn.example.com/ffmpeg/ffmpeg-core.wasm',
  },
  onStatus: (msg, { level }) => {},       // level: 'loading' | 'info'
  onError:  (err) => {},
  onReady:  (info) => {},                 // { videoCodec, audioCodec, subtitleCount, durationMs }
  onTracks: (tracks) => {},
});

await player.load('https://media.example.com/video.mkv');
// …
player.destroy();
```

`createPlayer(container, opts)` builds the `<video-player>` markup into `container` and
returns `{ load(url, opts?), destroy(), on(event, fn), off(event, fn), video }`.

### Controls

`controls` is a preset name or an object. Presets: `full` (everything), `minimal`
(play + time slider + volume + fullscreen + hotkeys/gestures), `none` (bare video, no
control bar). The object form merges per-control booleans onto a preset:

```js
controls: { preset: 'minimal', chapters: true, subtitles: true }
```

Per-control keys: `play`, `seek`, `chapterSkip`, `timeSlider`, `chapterMarkers`, `chapters`,
`audio`, `subtitles`, `volume`, `fullscreen`, `hotkeys`, `gestures`.

`dock` controls where the bar sits: `'overlay'` (default) draws it over the bottom of the video —
flush to the edges in the windowed player, a rounded floating pill in fullscreen. `'below'` docks
it *under* the video image (rounded, always visible, no overlap with subtitles); in fullscreen it
falls back to the overlay pill.

```js
controls: { preset: 'full', dock: 'below' }
```

### ffmpeg core (audio transcoding)

ffmpeg.wasm is used only to transcode **audio** tracks the browser can't decode natively in the
remuxed fMP4 (video is always remuxed by the `mkv-player` WASM — ffmpeg never touches video). The
transcoder outputs **AAC-LC** (preferred — encodes reliably, universal MSE incl. Safari) or **Opus**
(fallback) — both royalty-free (AAC-LC's core patents have expired). Never a GPL/patent video codec.

**The bundled core is a custom build, not `@ffmpeg/core`.** The stock npm core is built
`--enable-gpl --enable-nonfree`, which is non-redistributable and patent-encumbered. Instead,
`ffmpeg-core/` compiles our own **LGPL, audio-only** core (all native/BSD codecs — AGPL-compatible,
no x264/x265). It decodes Vorbis/Opus/FLAC/ALAC/WavPack/TTA/PCM plus AAC-LC, AC-3, E-AC-3 and DTS
core, and encodes AAC-LC/Opus/FLAC. Copyleft is clean (LGPL); the included lossy codecs are either
royalty-free or have expired core patents (AAC-LC, AC-3, DTS core) — **except E-AC-3, which is newer
and may still be patented in your jurisdiction**. HE-AAC/xHE-AAC, TrueHD/MLP and DTS-HD are excluded.

```sh
npm run build:ffmpeg                       # builds the default `free-audio` core (Docker; slow once)
```

This is a **separate, cached** step — it isn't part of the app build. `scripts/setup-ffmpeg-core.mjs`
then copies the built `ffmpeg-core/dist/<profile>/` (core + `LICENSE` + `SOURCE.md`, the LGPL source
offer) into the consuming app's `public/ffmpeg/`, served same-origin. If the core hasn't been built,
the setup script warns and skips — the app still runs, transcoding just stays off.

Need the remaining still-patented lossless codecs (TrueHD/MLP, DTS-HD)? Build the opt-in `full`
profile yourself, accepting the patent responsibility — see `ffmpeg-core/README.md`. The shipped
default stays `free-audio`.

`ffmpeg.coreURL` / `ffmpeg.wasmURL` let a consumer point at any host (fetched via `toBlobURL`, so a
cross-origin host must send permissive CORS); when omitted they default to
`${baseURL}ffmpeg/ffmpeg-core.js` (same-origin). `transcode: false` disables transcoding at runtime;
a `TRANSCODE=off` build (with the `ffmpeg-stub.js` alias) strips ffmpeg from the bundle entirely.
AAC-LC is the universal fallback output, so transcoding works even where Opus-in-MP4 isn't
supported (e.g. Safari).

## Required consumer Vite config

The library is shipped as ESM **source** — the consuming app's bundler builds it as part of
its own graph. Because `mkv-player`, `jassub`, and `@ffmpeg/ffmpeg` resolve their `.wasm` and
workers via `new URL(…, import.meta.url)` / module workers, the consuming app's `vite.config.js`
**must** keep:

```js
optimizeDeps: {
  exclude: ['mkv-player', 'jassub', '@ffmpeg/ffmpeg', '@ffmpeg/util'],
  include: ['throughput', 'rvfc-polyfill'],
},
worker: { format: 'es' },
server: { fs: { allow: ['..'] } },   // to import the workspace packages
```

Because `optimizeDeps.include` resolves from the app root, the app must also declare
`jassub` as a direct dependency (not only rely on it as a transitive dep of this library) so
npm hoists jassub's CJS deps (`throughput`, `rvfc-polyfill`) where esbuild can find them.

To fully drop ffmpeg from a build (in addition to `transcode: false`), alias `@ffmpeg/ffmpeg`
and `@ffmpeg/util` to `mkv-player-ui/src/ffmpeg-stub.js` and define `__TRANSCODE__` as
`false` in your bundler config (a `resolve.alias` plus the `define`).

## Test

```sh
npm test   # node --test src/mse.test.js
```
