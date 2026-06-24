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
and seeking. (ASS subtitle rendering via [libass](https://github.com/libass/libass)
is planned; for now `S_TEXT/ASS` tracks are listed but not rendered.)

The WASM side extracts encoded frames (copying length-prefixed NALs straight through),
reconstructs DTS/composition offsets for B-frames, and writes fMP4 init + media
segments. Seeking uses the Matroska `Cues` index, with all positions anchored to the
Segment element's content-start offset. A cold-start access speculatively fetches the
first 32 KB and last 256 KB in parallel to warm the cache.

## What's in here

This is a Cargo workspace + a small web frontend:

| Crate / dir   | What it is                                                                                     |
| ------------- | ---------------------------------------------------------------------------------------------- |
| `ebml-spec`   | A proc-macro that ingests the official EBML/Matroska schema XML **at compile time**, so every element ID knows its name and type. |
| `ebml-wasm`   | The EBML reader **and the MKV→fMP4 remuxer**, exposed to JS via `wasm-bindgen`. The reader is a forward-only **async iterator** over `EbmlElement`s; on top of it `MatroskaPlayer` (in `player.rs`) exposes `open(url)`, `tracks()`, `init_segment()`, `media_segment()`, `cue_offset()`, `subtitles()`, and `cue_times()`. The fMP4 box writer lives in `fmp4.rs`, block/sample extraction in `remux.rs`, the seek index in `index.rs`, and track/codec mapping in `track.rs`. |
| `player-web`  | The **MKV player** frontend: [video.js v10](https://www.npmjs.com/package/@videojs/html) (`@videojs/html`) for the UI, driving MSE from the WASM remuxer, with a custom audio-track selector and native WebVTT subtitle tracks. |
| `matroska-web`| A browser demo UI: drop a local video file and explore its EBML structure as a tree with a hex inspector, plus a quick metadata summary (tracks, languages, resolution, duration). |

## Build & run

You'll need the Rust toolchain and [`wasm-pack`](https://rustwasm.github.io/wasm-pack/).

### The player (`player-web`)

```sh
# 1. Build the WASM remuxer (outputs ebml-wasm/pkg, consumed by player-web).
cd ebml-wasm
wasm-pack build --target web

# 2. Serve the sample media with Range support + CORS on :8501.
#    (player-web defaults to http://localhost:8501/example/toaru.mkv)
npm start            # simple-http-server -i --cors --port 8501

# 3. In another shell, run the player dev server.
cd ../player-web
npm install
npm run dev          # open the printed Vite URL
```

Put `.mkv` files under `ebml-wasm/example/` and point the URL box at them. Codecs the
browser can't decode (e.g. HEVC in Firefox, AC-3 in Chrome/Firefox) are listed but
flagged unsupported.

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
cargo run -p ebml-wasm --example dump_segments -- ebml-wasm/example/toaru.mkv /tmp/out
```

## License

**GNU Affero General Public License v3.0** (AGPL-3.0) — see [`LICENSE.txt`](LICENSE.txt).

You're free to use, study, modify, and share this. The catch: if you distribute
it **or run a modified version as a network service**, you must release your
source under the same license. In short — fork it all you want, but your changes
stay open too.
