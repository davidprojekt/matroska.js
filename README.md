# matroska.js

An experimental Matroska / EBML toolkit written in Rust, compiled to WebAssembly
so it can parse `.mkv` / `.webm` files **directly in the browser** — nothing is
uploaded.

> ⚠️ **Early prototype.** APIs are unstable, some EBML data types are still
> mis-decoded (notably floats), and there are rough edges throughout. Expect bugs.

## Goal

A full MKV web player. The container is parsed in Rust/WASM and **remuxed on the fly
into fragmented MP4 (fMP4)** that is fed to a `<video>` element through Media Source
Extensions — no transcoding, nothing uploaded. The browser plays it as long as the
underlying codecs are web-compatible, with **switchable audio and subtitle tracks**
and seeking. **ASS/SSA subtitles are rendered via [libass](https://github.com/libass/libass)
(through [JASSUB](https://github.com/ThaUnknown/jassub)) and work well.** (Plain-text /
WebVTT subtitle tracks are **not currently working**.)

The WASM side extracts encoded frames (copying length-prefixed NALs straight through),
reconstructs DTS/composition offsets for B-frames, and writes fMP4 init + media
segments. Seeking uses the Matroska `Cues` index, with all positions anchored to the
Segment element's content-start offset. A cold-start access speculatively fetches the
first 32 KB and last 256 KB in parallel to warm the cache.

## What's in here

This is a Cargo workspace + a small web frontend:

| Crate / dir   | What it is                                                                                     |
| ------------- | ---------------------------------------------------------------------------------------------- |
| `ebml-spec`   | A proc-macro that ingests the official EBML/Matroska schema XML **at compile time**, so every element ID knows its name and type. (MIT) |
| `ebml-wasm`   | The **EBML/Matroska parser core**: a forward-only **async iterator** over `EbmlElement`s plus the byte sources (`FetchSource`, `FsSource`, `MemSource`), exposed to JS via `wasm-bindgen` (`EbmlReader`, `FetchSourceEbml`). Container parsing only — no remuxing. (MIT) |
| `mkv-player`  | The **MKV→fMP4 remuxer and player**, built on `ebml-wasm` and exposed to JS via `wasm-bindgen`. `MatroskaPlayer` (in `player.rs`) exposes `open(url)`, `tracks()`, `init_segment()`, `media_segment()`, `cue_offset()`, `subtitles()`, and `cue_times()`. The fMP4 box writer lives in `fmp4.rs`, block/sample extraction in `remux.rs`, the seek index in `index.rs`, track/codec mapping in `track.rs`, and the streaming byte source in `stream_source.rs`. (AGPL-3.0) |
| `player-web`  | The **MKV player demo**: [video.js v10](https://www.npmjs.com/package/@videojs/html) (`@videojs/html`) for the UI, driving MSE from the `mkv-player` WASM remuxer, with a custom audio-track selector and ASS/SSA subtitle rendering via libass (JASSUB). File picker, URL box, and WebTorrent streaming. (AGPL-3.0) |
| `player-embed`| A **headless embeddable** build of the player: no chrome, just the video filling the frame. Loads the video given in the embedding URL (`?src=`), so it can be dropped into an `<iframe>`. Shares the `mkv-player` core with `player-web` (sans WebTorrent). (AGPL-3.0) |
| `matroska-web`| A browser demo UI: drop a local video file and explore its EBML structure as a tree with a hex inspector, plus a quick metadata summary (tracks, languages, resolution, duration). Uses only the `ebml-wasm` parser. |

## Supported codecs

The remuxer copies encoded frames straight into fMP4 — it never transcodes. A track
plays only if **both** the remuxer can wrap its codec (below) **and** the browser can
decode that codec in MP4 via Media Source Extensions. Tracks the remuxer can't wrap are
listed as *not muxable*; tracks it wraps but the browser can't decode are flagged
*unsupported* (e.g. HEVC in Firefox, AC-3 in Chrome/Firefox).

| Kind      | Matroska `CodecID`                 | Codec                | fMP4 sample entry |
| --------- | ---------------------------------- | -------------------- | ----------------- |
| Video     | `V_MPEG4/ISO/AVC`                  | H.264 / AVC          | `avc1`            |
| Video     | `V_MPEGH/ISO/HEVC`                 | H.265 / HEVC         | `hvc1`            |
| Video     | `V_VP9`                            | VP9                  | `vp09`            |
| Video     | `V_AV1`                            | AV1                  | `av01`            |
| Audio     | `A_AAC`                            | AAC                  | `mp4a`            |
| Audio     | `A_OPUS`                           | Opus                 | `Opus`            |
| Audio     | `A_AC3`                            | AC-3                 | `ac-3`            |
| Audio     | `A_EAC3`                           | E-AC-3               | `ec-3`            |
| Audio     | `A_FLAC`                           | FLAC                 | `fLaC`            |
| Audio     | `A_MPEG/L3`                        | MP3                  | `mp4a` (`.6B`)    |
| Subtitle  | `S_TEXT/ASS`, `S_TEXT/SSA`         | rendered via libass (JASSUB) | (extracted)   |
| Subtitle  | `S_TEXT/UTF8`, `S_TEXT/WEBVTT`, `S_TEXT/ASCII` | text → WebVTT — **not working yet** | (extracted) |

Anything else (DTS, TrueHD, Vorbis, PCM; MPEG-2/older video) is not muxable today and
would need transcoding (planned via ffmpeg-wasm). MP3 frame durations assume MPEG-1
Layer III (1152 samples/frame); the rarer MPEG-2/2.5 Layer III (576) is not yet
distinguished.

## Build & run

You'll need the Rust toolchain and [`wasm-pack`](https://rustwasm.github.io/wasm-pack/).

### The player (`player-web`)

```sh
# 1. Build the WASM remuxer (outputs mkv-player/pkg, consumed by player-web).
cd mkv-player
wasm-pack build --target web

# 2. Serve the sample media with Range support + CORS on :8501.
#    (player-web defaults to http://localhost:8501/example/toaru.mkv)
cd ../ebml-wasm
npm start            # simple-http-server -i --cors --port 8501

# 3. In another shell, run the player dev server.
cd ../player-web
npm install
npm run dev          # open the printed Vite URL
```

> `npm run build` in `player-web` / `player-embed` runs the `mkv-player` wasm build
> for you, so step 1 is only needed for the dev server.

Put `.mkv` files under `ebml-wasm/example/` and point the URL box at them. Codecs the
browser can't decode (e.g. HEVC in Firefox, AC-3 in Chrome/Firefox) are listed but
flagged unsupported.

### The embeddable player (`player-embed`)

A chrome-free build that plays whatever video URL is passed in the page URL — meant to be
hosted once and dropped into an `<iframe>` by anyone.

```sh
# Build the WASM remuxer first (mkv-player, step 1 above), then:
cd player-embed
npm install
npm run dev          # dev server
npm run build        # → dist/, deploy as a static site
```

Point an embedder at the deployed page and pass the video URL as `?src=` (URL-encoded):

```html
<iframe
  src="https://your-host.example/?src=https%3A%2F%2Fmedia.example.com%2Fvideo.mkv"
  width="800" height="450" allowfullscreen></iframe>
```

`?url=` is accepted as an alias, and a Base64-encoded `#hash` works too (matching the
demo page's "Copy" link). The video server must support HTTP **byte ranges** and, if it's
a different origin, send **CORS** headers (`Access-Control-Allow-Origin`) — the embed
preflights and shows a clear message otherwise. Add `allowfullscreen` to the iframe so the
fullscreen button works.

### The EBML inspector (`matroska-web`)

```sh
# from ebml-wasm/: build the wasm module and copy it into the inspector
cd ebml-wasm
wasm-pack build --target web && cp -r ./pkg ../matroska-web/

# then serve the frontend (any static server works)
cd ../matroska-web
npx simple-http-server -i --cors --port 8501
# open http://localhost:8501 and drop in an .mkv / .webm file
```

### Validating the remuxer without a browser

```sh
# Dumps init + first media segment per muxable track for ffprobe/mp4box to check.
cargo run -p mkv-player --example dump_segments -- ebml-wasm/example/toaru.mkv /tmp/out
```

## License

This repository is split by component:

- **`ebml-spec` and `ebml-wasm`** — the EBML/Matroska **parser core** — are licensed
  under the **MIT License** (see [`ebml-wasm/LICENSE`](ebml-wasm/LICENSE)). Use them
  freely, including in closed-source projects.
- **`mkv-player`** (the MKV→fMP4 remuxer/player) and the **player frontends**
  (`player-web`, `player-embed`) are licensed under the **GNU Affero General Public
  License v3.0** (AGPL-3.0) — see [`LICENSE.txt`](LICENSE.txt). You're free to use,
  study, modify, and share them, but if you distribute them **or run a modified
  version as a network service**, you must release your source under the same license.
