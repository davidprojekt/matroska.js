// In-browser audio transcoding via ffmpeg.wasm. This is the ONLY module that imports
// @ffmpeg/* — everything else reaches it through a dynamic import gated on the
// __TRANSCODE__ build flag, so a `TRANSCODE=off` build tree-shakes ffmpeg out entirely.
//
// It takes the self-contained Matroska chunks produced by the WASM core's `audio_chunk`
// (raw frames of a codec MSE can't decode) and re-encodes them to a fragmented MP4 that
// the existing MSE pipeline can append. The single-thread core is used on purpose: no
// SharedArrayBuffer means no COOP/COEP cross-origin isolation, which would break this
// embeddable/iframed player.
//
// Output codec is AAC-LC, not Opus: the default @ffmpeg/core's `libopus` encoder crashes
// in wasm ("memory access out of bounds") on every input, while the native `aac` encoder
// works and `audio/mp4; codecs="mp4a.40.2"` is universally MSE-supported. A custom core
// with a working Opus encoder could switch OUTPUT_MIME/ENCODE_ARGS back to Opus.

import { FFmpeg } from '@ffmpeg/ffmpeg';
import { toBlobURL } from '@ffmpeg/util';

// The core is served same-origin from public/ffmpeg/ (see scripts/setup-ffmpeg-core.mjs).
const base = import.meta.env.BASE_URL || '/';
const CORE_URL = `${base}ffmpeg/ffmpeg-core.js`;
const WASM_URL = `${base}ffmpeg/ffmpeg-core.wasm`;

// empty_moov + default_base_moof make each fragment self-contained (carries its own init),
// so windows can be appended independently; the input chunk is zero-anchored, so output
// starts at 0 and the caller places it with SourceBuffer.timestampOffset. -ac 2 keeps every
// fragment's init identical (MSE requires matching init segments across re-appends).
const OUTPUT = 'out.mp4';
const ENCODE_ARGS = [
  '-map', '0:a:0',
  '-c:a', 'aac', '-b:a', '192k', '-ac', '2',
  '-movflags', '+frag_keyframe+empty_moov+default_base_moof',
  '-muxpreload', '0', '-muxdelay', '0',
  '-f', 'mp4',
];

// The MSE mime for the transcoded output. Exported so the SourceBuffer is created with
// the *output* codec, not the (unsupported) source codec.
export const TRANSCODE_MIME = 'audio/mp4; codecs="mp4a.40.2"';

// Emscripten MEMFS never shrinks; rebuild the instance periodically to reclaim its heap
// over a long movie (each window writes+reads ~hundreds of KB).
const RECREATE_EVERY = 40;

export class AudioTranscoder {
  constructor({ onLog } = {}) {
    this.ffmpeg = null;
    this.loadPromise = null;
    this.tail = Promise.resolve(); // serializes exec()s — one per instance at a time
    this.seq = 0;
    this.runs = 0;
    this.onLog = onLog;
    this.dead = false;
  }

  /** MSE mime of the transcoded output — the SourceBuffer is created with this. */
  get mime() {
    return TRANSCODE_MIME;
  }

  async ensureLoaded() {
    if (this.ffmpeg) return this.ffmpeg;
    if (!this.loadPromise) {
      this.loadPromise = (async () => {
        const ff = new FFmpeg();
        if (this.onLog) ff.on('log', ({ message }) => this.onLog(message));
        await ff.load({
          coreURL: await toBlobURL(CORE_URL, 'text/javascript'),
          wasmURL: await toBlobURL(WASM_URL, 'application/wasm'),
        });
        this.ffmpeg = ff;
        this.loadPromise = null;
        return ff;
      })();
    }
    return this.loadPromise;
  }

  /**
   * Transcode one Matroska chunk (Uint8Array) → fragmented Opus/MP4 (Uint8Array).
   * Serialized: a single ffmpeg instance can only run one exec at a time.
   */
  transcode(mkvBytes) {
    const run = this.tail.then(() => this._runOne(mkvBytes));
    this.tail = run.catch(() => {}); // keep the chain alive past a failed run
    return run;
  }

  async _runOne(mkvBytes) {
    if (this.dead) throw new Error('transcoder destroyed');
    const ff = await this.ensureLoaded();
    const input = `in_${this.seq++}.mkv`;
    try {
      await ff.writeFile(input, mkvBytes);
      await ff.exec(['-i', input, ...ENCODE_ARGS, OUTPUT]);
      const data = await ff.readFile(OUTPUT); // throws if exec produced no output
      return data instanceof Uint8Array ? data : new Uint8Array(data);
    } finally {
      try { await ff.deleteFile(input); } catch (_) {}
      try { await ff.deleteFile(OUTPUT); } catch (_) {}
      this._maybeRecycle();
    }
  }

  _maybeRecycle() {
    if (++this.runs < RECREATE_EVERY) return;
    this.runs = 0;
    const old = this.ffmpeg;
    this.ffmpeg = null; // next transcode reloads a fresh instance
    try { old?.terminate(); } catch (_) {}
  }

  destroy() {
    this.dead = true;
    const old = this.ffmpeg;
    this.ffmpeg = null;
    this.loadPromise = null;
    try { old?.terminate(); } catch (_) {}
  }
}
