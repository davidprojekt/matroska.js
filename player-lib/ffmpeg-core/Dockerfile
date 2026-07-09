# syntax=docker/dockerfile:1
#
# Builds a custom single-thread ffmpeg.wasm core for the MKV player's audio transcoder.
#
# Trimmed + podman-compatible fork of ffmpegwasm/ffmpeg.wasm's Dockerfile (pinned via build.sh):
#  - only the ROYALTY-FREE / LGPL libraries are compiled (libogg, libopus, libvorbis, zlib);
#    the GPL/patent-encumbered stages (x264, x265, libvpx, lame, theora, webp, freetype, fribidi,
#    harfbuzz, libass, zimg) are dropped entirely,
#  - ffmpeg is configured WITHOUT --enable-gpl / --enable-nonfree and with --disable-everything,
#    then a free audio whitelist is re-enabled (see FFMPEG_CONF / free-audio.env),
#  - `ADD <git-url>#branch` (a BuildKit-only feature) is replaced with `RUN git clone` so this
#    builds under podman/buildah, not just docker buildx.
#
# Pinned toolchain: emscripten 3.1.40, ffmpeg n5.1.4 (matches ffmpeg.wasm v0.12.10 / tag v12.15).
# Output: /dist/esm/ffmpeg-core.{js,wasm} + /dist/licenses/* (extracted from the real sources).

FROM docker.io/emscripten/emsdk:3.1.40 AS emsdk-base
ARG EXTRA_CFLAGS=""
ARG EXTRA_LDFLAGS=""
ARG FFMPEG_ST=yes
ENV INSTALL_DIR=/opt
ENV FFMPEG_VERSION=n5.1.4
ENV CFLAGS="-I$INSTALL_DIR/include $EXTRA_CFLAGS"
ENV CXXFLAGS="$CFLAGS"
ENV LDFLAGS="-L$INSTALL_DIR/lib $CFLAGS $EXTRA_LDFLAGS"
ENV EM_PKG_CONFIG_PATH=$INSTALL_DIR/lib/pkgconfig:/emsdk/upstream/emscripten/system/lib/pkgconfig
ENV PKG_CONFIG_PATH=$EM_PKG_CONFIG_PATH
ENV EM_TOOLCHAIN_FILE=$EMSDK/upstream/emscripten/cmake/Modules/Platform/Emscripten.cmake
ENV FFMPEG_ST=$FFMPEG_ST
RUN apt-get update && apt-get install -y pkg-config autoconf automake libtool ragel git

# --- Royalty-free libraries only (all BSD/permissive) -----------------------------------------

FROM emsdk-base AS ogg-builder
RUN git clone --depth 1 -b v1.3.4 https://github.com/ffmpegwasm/Ogg.git /src
COPY build/ogg.sh /src/build.sh
RUN bash -x /src/build.sh

FROM emsdk-base AS opus-builder
RUN git clone --depth 1 -b v1.3.1 https://github.com/ffmpegwasm/opus.git /src
COPY build/opus.sh /src/build.sh
RUN bash -x /src/build.sh

FROM emsdk-base AS vorbis-builder
COPY --from=ogg-builder $INSTALL_DIR $INSTALL_DIR
RUN git clone --depth 1 -b v1.3.3 https://github.com/ffmpegwasm/vorbis.git /src
COPY build/vorbis.sh /src/build.sh
RUN bash -x /src/build.sh

FROM emsdk-base AS zlib-builder
RUN git clone --depth 1 -b v1.2.11 https://github.com/ffmpegwasm/zlib.git /src
COPY build/zlib.sh /src/build.sh
RUN bash -x /src/build.sh

# --- ffmpeg -----------------------------------------------------------------------------------

FROM emsdk-base AS ffmpeg-base
RUN embuilder build sdl2
RUN git clone --depth 1 -b $FFMPEG_VERSION https://github.com/FFmpeg/FFmpeg.git /src
COPY --from=ogg-builder $INSTALL_DIR $INSTALL_DIR
COPY --from=opus-builder $INSTALL_DIR $INSTALL_DIR
COPY --from=vorbis-builder $INSTALL_DIR $INSTALL_DIR
COPY --from=zlib-builder $INSTALL_DIR $INSTALL_DIR

FROM ffmpeg-base AS ffmpeg-builder
# The full configure line (LGPL, --disable-everything + a royalty-free audio whitelist) is passed
# from the selected profile (see build.sh / profiles/*.env). NO --enable-gpl / --enable-nonfree.
ARG FFMPEG_CONF
COPY build/ffmpeg.sh /src/build.sh
RUN bash -x /src/build.sh ${FFMPEG_CONF}

FROM ffmpeg-builder AS ffmpeg-wasm-builder
# Only the free libraries are linked.
ARG FFMPEG_LINK_LIBS
ENV FFMPEG_LIBS=${FFMPEG_LINK_LIBS}
COPY src/bind /src/src/bind
COPY src/fftools /src/src/fftools
COPY build/ffmpeg-wasm.sh build.sh
RUN mkdir -p /src/dist/esm && bash -x /src/build.sh \
      ${FFMPEG_LIBS} \
      -sEXPORT_ES6 \
      -o dist/esm/ffmpeg-core.js

# --- Export: core assets + the actual license texts from the sources --------------------------
FROM scratch AS exportor
COPY --from=ffmpeg-wasm-builder /src/dist/esm /esm
COPY --from=ffmpeg-builder /src/COPYING.LGPLv2.1 /licenses/ffmpeg-COPYING.LGPLv2.1.txt
COPY --from=ffmpeg-builder /src/COPYING.LGPLv3 /licenses/ffmpeg-COPYING.LGPLv3.txt
COPY --from=opus-builder /src/COPYING /licenses/opus-COPYING.txt
COPY --from=vorbis-builder /src/COPYING /licenses/vorbis-COPYING.txt
COPY --from=ogg-builder /src/COPYING /licenses/ogg-COPYING.txt
COPY --from=zlib-builder /src/README /licenses/zlib-README-license.txt
