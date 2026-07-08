# mkv-player-ui

The reusable browser MKV player extracted from `player-web` and `player-embed`. It parses
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

### ffmpeg core

`ffmpeg.coreURL` / `ffmpeg.wasmURL` point at any host (they're fetched via `toBlobURL`, so a
cross-origin CDN must send permissive CORS). When omitted they default to
`${baseURL}ffmpeg/ffmpeg-core.js` (same-origin), and `scripts/setup-ffmpeg-core.mjs` copies
the single-thread core into the consuming app's `public/ffmpeg/`. Set `transcode: false` to
disable transcoding at runtime.

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
`false`. See `player-embed/vite.config.js` for the reference setup.

## Test

```sh
npm test   # node --test src/mse.test.js
```
