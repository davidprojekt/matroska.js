#!/bin/bash
# Patched copy of ffmpeg.wasm's build/ffmpeg-wasm.sh, overlaid into the build context by build.sh.
# The only change from upstream: drop `-Llibpostproc`/`-lpostproc`. libpostproc requires
# `--enable-gpl`, which our royalty-free LGPL build deliberately omits, so libpostproc.a is never
# produced and the upstream hardcoded `-lpostproc` breaks the link.
#
# `-o <OUTPUT_FILE_NAME>` must be provided when using this build script.
set -euo pipefail

EXPORT_NAME="createFFmpegCore"

CONF_FLAGS=(
  -I.
  -I./src/fftools
  -I$INSTALL_DIR/include
  -L$INSTALL_DIR/lib
  -Llibavcodec
  -Llibavdevice
  -Llibavfilter
  -Llibavformat
  -Llibavutil
  -Llibswresample
  -Llibswscale
  -lavcodec
  -lavdevice
  -lavfilter
  -lavformat
  -lavutil
  -lswresample
  -lswscale
  -Wno-deprecated-declarations
  $LDFLAGS
  -sENVIRONMENT=worker
  -sWASM_BIGINT
  -sUSE_SDL=2
  -sMODULARIZE
  ${FFMPEG_MT:+ -sINITIAL_MEMORY=1024MB}
  ${FFMPEG_MT:+ -sPTHREAD_POOL_SIZE=32}
  ${FFMPEG_ST:+ -sINITIAL_MEMORY=32MB -sALLOW_MEMORY_GROWTH}
  -sEXPORT_NAME="$EXPORT_NAME"
  -sEXPORTED_FUNCTIONS=$(node src/bind/ffmpeg/export.js)
  -sEXPORTED_RUNTIME_METHODS=$(node src/bind/ffmpeg/export-runtime.js)
  -lworkerfs.js
  --pre-js src/bind/ffmpeg/bind.js
  src/fftools/cmdutils.c
  src/fftools/ffmpeg.c
  src/fftools/ffmpeg_filter.c
  src/fftools/ffmpeg_hw.c
  src/fftools/ffmpeg_mux.c
  src/fftools/ffmpeg_opt.c
  src/fftools/opt_common.c
  src/fftools/ffprobe.c
)

emcc "${CONF_FLAGS[@]}" $@
