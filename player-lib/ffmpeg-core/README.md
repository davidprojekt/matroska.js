# ffmpeg-core — custom ffmpeg.wasm build for the audio transcoder

The player transcodes **audio** tracks the browser can't play natively in the remuxed fMP4 (video
is always remuxed by the Rust `mkv-player` WASM — ffmpeg never touches video here). The stock
`@ffmpeg/core` on npm is built `--enable-gpl --enable-nonfree`, which is **non-redistributable** and
carries patent-encumbered codecs, so it can't be shipped. This directory compiles our own core.

## Default: `free-audio` (LGPL, audio-only)

`npm run build:ffmpeg` builds the `free-audio` profile: FFmpeg **LGPL** (no `--enable-gpl`, no
`--enable-nonfree`), `--disable-everything` + an audio whitelist — decode
Vorbis/Opus/FLAC/ALAC/WavPack/TTA/PCM plus **AAC-LC, AC-3, E-AC-3, DTS core**, encode
**AAC-LC** / **Opus** / FLAC, demux Matroska, mux fragmented MP4. All are ffmpeg's native codecs or
BSD libs (no external nonfree lib, no `--enable-gpl`), so the binary is **LGPL / AGPL-compatible**.

Patent posture (separate from copyleft): Opus/Vorbis/FLAC/PCM are royalty-free; **AAC-LC, AC-3 and
DTS core** have expired core patents / are commonly treated as license-free; **E-AC-3 is newer and there has been no official verdict that proves it's royalty-free**. HE-AAC/
xHE-AAC, TrueHD/MLP and DTS-HD are excluded (see `profiles/full.env.example`). See
`profiles/free-audio.env`. Only one demuxer (Matroska) and one muxer (MP4) are built: the WASM core
feeds ffmpeg a self-contained Matroska chunk of raw audio, and MSE needs the fragmented-MP4 output.

Built **without wasm SIMD** (`-O3`, no `-msimd128`): the ffmpegwasm libopus aborts with "memory
access out of bounds" under SIMD. Audio-only transcode is light, so the perf cost is negligible.

Output (gitignored) → `dist/free-audio/{ffmpeg-core.js,ffmpeg-core.wasm,LICENSE,SOURCE.md}`.
`SOURCE.md` records the pinned FFmpeg/Emscripten versions and exact configure line (LGPL source
offer); `LICENSE` bundles the real license texts from the sources.

## Bigger builds (opt-in, your responsibility)

Need the still-patented lossless codecs the default omits — **TrueHD/MLP, DTS-HD**? Copy
`profiles/full.env.example` → `profiles/full.env`, read the patent notice, then:

```sh
FFMPEG_PROFILE=full npm run build:ffmpeg
```

Distributing or operating a build that decodes those codecs may require patent licenses in your
jurisdiction — that responsibility is yours. The shipped default stays `free-audio`.

## How it works

`build.sh` clones `ffmpegwasm/ffmpeg.wasm` at a **pinned tag + verified commit SHA**, then builds a
trimmed, **podman-compatible** `Dockerfile` (only the free libraries; `RUN git clone` instead of
BuildKit's `ADD <git-url>`) via `podman build -o type=local` (or `docker buildx`). Single-thread
core only (no `SharedArrayBuffer`/COOP-COEP, so the player stays iframe-embeddable). First build is
slow (tens of minutes); layers cache after.

This is a **separate, cached** step — normal app builds don't run it; they stage whatever is in
`dist/` and disable transcoding if it's missing.
