# @matroska-js/remux

WebAssembly **MKV / Matroska → fragmented-MP4 remuxer**, compiled from Rust. It parses a `.mkv` /
`.webm` container directly in the browser and rewrites its tracks into fMP4 segments you feed to a
`<video>` element through Media Source Extensions — the video is **remuxed, never transcoded**, so
nothing is uploaded and no server-side conversion is involved.

This is the low-level engine behind
**[`@matroska-js/player`](https://www.npmjs.com/package/@matroska-js/player)**. If you want a
ready-made player — controls, subtitles, audio transcoding — use that instead. Reach for
`@matroska-js/remux` directly only when you're building your own MSE pipeline.

## Install

```sh
npm i @matroska-js/remux
```

## Usage

Built with `wasm-pack --target web`: import the default init function (which loads the `.wasm`),
then drive `MatroskaPlayer`.

```js
import init, { MatroskaPlayer } from '@matroska-js/remux';

await init();                                    // instantiate the wasm module
const player = await MatroskaPlayer.open(url);   // ranged HTTP reads — the server must support Range

// Per track: one fMP4 init segment, then media segments for a time window.
const initSeg = player.init_segment(trackNumber);                        // Uint8Array
const media   = await player.media_segment(trackNumber, startMs, endMs); // Uint8Array
// …append these to a MediaSource SourceBuffer.
```

The full API — track listing, cue-based seeking, chapters, font attachments, ASS/PGS subtitle
extraction, and audio-chunk export for transcoding — is documented in the bundled TypeScript
declarations (`matroska_remux.d.ts`).

### Bundler note

The package resolves its `.wasm` via `new URL('…', import.meta.url)`. With Vite, keep it out of
dependency pre-bundling so that reference survives and the asset is emitted:

```js
optimizeDeps: { exclude: ['@matroska-js/remux'] }
```

## Codecs

Remuxes web-decodable tracks — H.264/AVC, HEVC/H.265, VP9, AV1 video; AAC, Opus, AC-3, E-AC-3,
FLAC, MP3 audio — into fMP4. It never transcodes, so a track plays only if the browser can decode
that codec in MP4 via MSE. (In-browser audio transcoding for non-decodable audio is provided by
`@matroska-js/player`, not this package.)

## License

AGPL-3.0-only. © David Schneider. A separate commercial license — without the AGPL's
source-disclosure obligations — is available; contact **licensing@davidschneider.xyz**.
