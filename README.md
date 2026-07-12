# matroska.js

An experimental Matroska / EBML toolkit written in Rust, compiled to WebAssembly
so it can parse `.mkv` / `.webm` files **directly in the browser** — nothing is
uploaded.

**Live demos:** [play a `.mkv`](https://play.matroska.davidschneider.xyz) ·
[explore EBML structure](https://inspect.matroska.davidschneider.xyz)

> ⚠️ **Early prototype.** APIs are unstable,
> there are rough edges throughout. Expect bugs.

## Goal

A full MKV web player. The container is parsed in Rust/WASM and **remuxed on the fly
into fragmented MP4 (fMP4)** that is fed to a `<video>` element through Media Source
Extensions — no upload, and video is never transcoded. The browser plays it as long as
the video codec is web-compatible, with **switchable audio and subtitle tracks**,
chapters, and seeking. Audio in a codec the browser can't decode is **transcoded
in-browser** by a royalty-free [ffmpeg.wasm](https://ffmpegwasm.netlify.app/) core
(offline-first, no CDN dependency); video still plays untouched.

Two subtitle families are rendered over a canvas overlay, and up to **two can show at
once** (e.g. dialogue + signs):

- **ASS/SSA** via [libass](https://github.com/libass/libass) (through
  [JASSUB](https://github.com/ThaUnknown/jassub)), with embedded fonts.
- **PGS** (`S_HDMV/PGS`, Blu-ray bitmap subtitles) via
  [libpgs](https://github.com/Arcus92/libpgs-js).

Per-block zlib-compressed subtitle tracks (mkvmerge's default) are decompressed
transparently. Plain-text / WebVTT subtitle tracks are **not currently working**;
DVD (VobSub) and DVBSUB bitmap subtitles are **not yet supported**.

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
| `mkv-player`  | The **MKV→fMP4 remuxer and player**, built on `ebml-wasm` and exposed to JS via `wasm-bindgen`. `MatroskaPlayer` (in `player.rs`) exposes `open(url)`, `tracks()`, `init_segment()`, `media_segment()`, `audio_chunk()` (for transcoding), `cue_offset()`, `cue_times()`, `chapters()`, `font_attachments()`, and the subtitle accessors `subtitles()` (text→WebVTT), `subtitle_header()` / `subtitle_events()` (ASS), and `subtitle_bitmap_events()` (PGS→`.sup`). The fMP4 box writer lives in `fmp4.rs`, block/sample extraction and subtitle-block decompression in `remux.rs` / `track.rs`, the seek index in `index.rs`, and the streaming byte source in `stream_source.rs`. (AGPL-3.0) |
| `player-lib`  | **`@matroska-js/player`** — the **reusable player library** all frontends are built on: [video.js v10](https://www.npmjs.com/package/@videojs/html) (`@videojs/html`) UI driving MSE from the `mkv-player` WASM remuxer, ASS/SSA subtitles via libass (JASSUB), PGS subtitles via libpgs, and in-browser audio transcoding via a bundled royalty-free ffmpeg.wasm core. `createPlayer(container, opts)` builds the control bar (controls are configurable/removable). (AGPL-3.0) |
| `player-web`  | The **MKV player demo**: a URL box, local-file picker, and copy-shareable-link button around `@matroska-js/player`. (AGPL-3.0) |
| **[matroska-player-nextcloud](https://github.com/davidprojekt/matroska-player-nextcloud)** ↗ | A **[Nextcloud](https://nextcloud.com/) app** that plays `.mkv` / `.mka` files in the Files **Viewer** via `@matroska-js/player`. Lives in its **own repository** (built on the published npm package). (AGPL-3.0) |
| `matroska-inspector`| A browser demo UI: drop a local video file and explore its EBML structure as a tree with a hex inspector, plus a quick metadata summary (tracks, languages, resolution, duration). Uses only the `ebml-wasm` parser. (MIT) |

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
| Subtitle  | `S_HDMV/PGS`                       | Blu-ray bitmap, rendered via libpgs | (extracted, reconstructed `.sup`) |
| Subtitle  | `S_TEXT/UTF8`, `S_TEXT/WEBVTT`, `S_TEXT/ASCII` | text → WebVTT — **not working yet** | (extracted) |

Audio the browser can't decode natively (e.g. AC-3/E-AC-3 in some browsers) is
transcoded in-browser to AAC (or Opus) by the bundled ffmpeg.wasm core, so it plays
without a native decoder. Other audio codecs not listed above (DTS, TrueHD, Vorbis, PCM)
route through the same transcoder. Video is never transcoded, so an undecodable video
codec (e.g. HEVC in Firefox, or MPEG-2/older) is listed but won't play. MP3 frame
durations assume MPEG-1 Layer III (1152 samples/frame); the rarer MPEG-2/2.5 Layer III
(576) is not yet distinguished. DVD (VobSub) and DVBSUB bitmap subtitles are not yet
supported.

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

# 3. In another shell, install the shared player library, then run the dev server.
cd ../player-lib
npm install          # installs the library's deps (video.js, jassub, libpgs, ffmpeg.wasm)
cd ../player-web
npm install
npm run dev          # open the printed Vite URL
```

> The demo consumes the shared **`@matroska-js/player`** library in `player-lib/` (a
> workspace `file:` dependency), so run `npm install` in `player-lib` before it.
> `npm run build` in `player-web` runs the `mkv-player` wasm build
> for you, so step 1 is only needed for the dev server.

Put `.mkv` files under `ebml-wasm/example/` and point the URL box at them. Codecs the
browser can't decode (e.g. HEVC in Firefox, AC-3 in Chrome/Firefox) are listed but
flagged unsupported.

### The EBML inspector (`matroska-inspector`)

```sh
# from ebml-wasm/: build the wasm module and copy it into the inspector
cd ebml-wasm
wasm-pack build --target web && cp -r ./pkg ../matroska-inspector/

# then serve the frontend (any static server works)
cd ../matroska-inspector
npx simple-http-server -i --cors --port 8501
# open http://localhost:8501 and drop in an .mkv / .webm file
```

### Validating the remuxer without a browser

```sh
# Dumps init + first media segment per muxable track for ffprobe/mp4box to check.
cargo run -p matroska-remux --example dump_segments -- ebml-wasm/example/toaru.mkv /tmp/out
```

## License

This repository is split by component:

- **`ebml-spec`, `ebml-wasm`, and `matroska-inspector`** — the EBML/Matroska **parser core** and its
  browser demo — are licensed under the **MIT License** (see [`ebml-wasm/LICENSE`](ebml-wasm/LICENSE)).
  Use them freely, including in closed-source projects.
- **`mkv-player`** (the MKV→fMP4 remuxer/player), the **`@matroska-js/player`** library (`player-lib`),
  the **player demo** (`player-web`), and the **Nextcloud app**
  ([matroska-player-nextcloud](https://github.com/davidprojekt/matroska-player-nextcloud), separate repo) are licensed
  under the **GNU Affero General Public License v3.0** (AGPL-3.0) — see
  [`LICENSE.txt`](LICENSE.txt). You're free to use,
  study, modify, and share them, but if you distribute them **or run a modified
  version as a network service**, you must release your source under the same license.

### Commercial licensing

The AGPL components are also available under a **separate commercial license** — for embedding
`@matroska-js/player` in a closed-source or SaaS product without the AGPL's source-disclosure
obligations, or for a watermark-free build. The author holds full copyright and can grant such
terms. To arrange an agreement, contact **licensing@davidschneider.xyz**.
