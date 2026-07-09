#!/usr/bin/env bash
# Compile a custom single-thread ffmpeg.wasm core for the audio transcoder.
#
#   npm run build:ffmpeg                    # default: free-audio profile
#   FFMPEG_PROFILE=full npm run build:ffmpeg # opt-in bigger build (you provide full.env)
#
# Output (gitignored): ffmpeg-core/dist/<profile>/{ffmpeg-core.js,ffmpeg-core.wasm,LICENSE,SOURCE.md}
#
# Requires a container engine with local-output support: podman (this repo's default) or
# docker buildx. The core is compiled from source (Emscripten) — the first run is slow (tens of
# minutes) but layers cache. It is intentionally a SEPARATE step: the app builds don't run it, they
# just stage whatever is in dist/ (and disable transcoding if it's absent).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
profile="${FFMPEG_PROFILE:-free-audio}"
envfile="$here/profiles/$profile.env"

# Pinned upstream (ffmpeg.wasm v12.15 == @ffmpeg/core 0.12.10; emscripten 3.1.40, ffmpeg n5.1.4).
PIN_TAG="v12.15"
PIN_SHA="71aa99d37c02a7b4c435275ca9ef50e612f6efa1"
REPO="https://github.com/ffmpegwasm/ffmpeg.wasm"

[ -f "$envfile" ] || { echo "ERROR: profile '$profile' not found ($envfile)."; \
  echo "For a bigger build: cp profiles/full.env.example profiles/full.env  (read the patent notice)."; exit 1; }
# shellcheck disable=SC1090
source "$envfile"
: "${FFMPEG_CONF:?profile must set FFMPEG_CONF}"
: "${FFMPEG_LINK_LIBS:?profile must set FFMPEG_LINK_LIBS}"

# Pick a builder that supports local output.
if command -v podman >/dev/null 2>&1; then
  BUILD=(podman build)
elif docker buildx version >/dev/null 2>&1; then
  BUILD=(docker buildx build)
else
  echo "ERROR: need podman or 'docker buildx' to compile the ffmpeg core."; exit 1
fi

work="$(mktemp -d)"
export_dir="$(mktemp -d)"
trap 'rm -rf "$work" "$export_dir"' EXIT

echo "[ffmpeg-core] cloning $REPO@$PIN_TAG …"
git clone --depth 1 -b "$PIN_TAG" "$REPO" "$work" >/dev/null 2>&1
got="$(git -C "$work" rev-parse HEAD)"
[ "$got" = "$PIN_SHA" ] || { echo "ERROR: integrity check failed — $REPO@$PIN_TAG is $got, expected $PIN_SHA."; exit 1; }
echo "[ffmpeg-core] pinned commit verified ($PIN_SHA)."

# Overlay our patched link script (drops -lpostproc; see ffmpeg-wasm.sh) into the build context.
cp "$here/ffmpeg-wasm.sh" "$work/build/ffmpeg-wasm.sh"

echo "[ffmpeg-core] building '$profile' core (this is slow on first run)…"
"${BUILD[@]}" \
  -f "$here/Dockerfile" \
  --build-arg "FFMPEG_ST=yes" \
  `# No -msimd128: the ffmpegwasm libopus build aborts ("memory access out of bounds") when` \
  `# compiled with wasm SIMD, so we build scalar. Audio-only transcode is light; the perf cost` \
  `# is negligible and Opus then encodes reliably.` \
  --build-arg "EXTRA_CFLAGS=-O3" \
  --build-arg "FFMPEG_CONF=$FFMPEG_CONF" \
  --build-arg "FFMPEG_LINK_LIBS=$FFMPEG_LINK_LIBS" \
  -o "type=local,dest=$export_dir" \
  "$work"

[ -f "$export_dir/esm/ffmpeg-core.js" ] && [ -f "$export_dir/esm/ffmpeg-core.wasm" ] \
  || { echo "ERROR: build produced no core assets."; exit 1; }

out="$here/dist/$profile"
rm -rf "$out"; mkdir -p "$out"
cp "$export_dir/esm/ffmpeg-core.js" "$export_dir/esm/ffmpeg-core.wasm" "$out/"

# Assemble the license bundle from the real source texts + a source offer (LGPL compliance).
{
  echo "ffmpeg.wasm core ($profile profile) — component licenses"
  echo "======================================================="
  echo
  echo "FFmpeg n5.1.4, built LGPL (no --enable-gpl, no --enable-nonfree)."
  echo "Bundled libraries: libopus (BSD), libvorbis + libogg (BSD), zlib (Zlib) — all permissive."
  echo "Corresponding source and the exact build configuration: see SOURCE.md."
  echo
  for f in "$export_dir"/licenses/*.txt; do
    [ -f "$f" ] || continue
    echo; echo "===== $(basename "$f") ====="; echo; cat "$f"
  done
} > "$out/LICENSE"

cat > "$out/SOURCE.md" <<EOF
# ffmpeg.wasm core — corresponding source (LGPL offer)

This \`ffmpeg-core.js\` / \`ffmpeg-core.wasm\` is a WebAssembly build of FFmpeg, produced from
unmodified upstream sources at pinned versions. It is **LGPL-2.1-or-later** (configured **without**
\`--enable-gpl\` and **without** \`--enable-nonfree\`), audio-only, royalty-free.

## Sources
- FFmpeg **n5.1.4** — https://github.com/FFmpeg/FFmpeg/tree/n5.1.4
- libopus **v1.3.1**, libvorbis **v1.3.3**, libogg **v1.3.4**, zlib **v1.2.11**
  (via ffmpeg.wasm mirrors, pinned by tag $PIN_TAG / commit $PIN_SHA of
  https://github.com/ffmpegwasm/ffmpeg.wasm)
- Toolchain: Emscripten **3.1.40**

## Exact build configuration (profile: $profile)
\`\`\`
ffmpeg ./configure … $FFMPEG_CONF
link libs: $FFMPEG_LINK_LIBS
\`\`\`

## Rebuild
The complete, scripted build is \`ffmpeg-core/\` in the mkv-player-ui sources
(\`Dockerfile\`, \`build.sh\`, \`profiles/$profile.env\`). Run \`npm run build:ffmpeg\` to reproduce.
EOF

echo "[ffmpeg-core] done → $out"
ls -la "$out"
