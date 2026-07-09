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
// Output is ROYALTY-FREE. The bundled core is a custom LGPL, audio-only build (see ffmpeg-core/):
// Opus (preferred — smallest, unquestionably patent-free) with AAC-LC as a fallback. AAC-LC's core
// patents have expired, so it counts as royalty-free here, and it is universally MSE-appendable —
// including Safari, which supports neither Opus nor FLAC in MP4. The output codec is chosen at
// runtime from what MediaSource can actually append.

import { FFmpeg } from '@ffmpeg/ffmpeg';
import { toBlobURL } from '@ffmpeg/util';

// The ffmpeg core (glue JS + wasm) is loaded from URLs the caller supplies — see
// `createPlayer`'s `ffmpeg.coreURL`/`ffmpeg.wasmURL` options. Any origin works as long as
// it sends permissive CORS (toBlobURL fetches both). The apps default these to their own
// same-origin public/ffmpeg/ (populated by scripts/setup-ffmpeg-core.mjs).

// empty_moov + default_base_moof make each fragment self-contained (carries its own init),
// so windows can be appended independently; the input chunk is zero-anchored, so output
// starts at 0 and the caller places it with SourceBuffer.timestampOffset. -ac 2 keeps every
// fragment's init identical (MSE requires matching init segments across re-appends).
const OUTPUT = 'out.mp4';
const MUX_ARGS = [
  '-movflags', '+frag_keyframe+empty_moov+default_base_moof',
  '-muxpreload', '0', '-muxdelay', '0',
  '-f', 'mp4',
];

// Output candidates, best first. AAC-LC (its core patents have expired) is preferred: it encodes
// reliably in wasm and is universally MSE-appendable, including Safari. Opus is the fallback —
// smaller and unquestionably royalty-free, and it encodes fine now the core is built without wasm
// SIMD (see ffmpeg-core/build.sh; the ffmpegwasm libopus aborts under -msimd128).
const OUTPUTS = [
  { mime: 'audio/mp4; codecs="mp4a.40.2"', codec: ['-c:a', 'aac', '-b:a', '192k', '-ac', '2'] },
  { mime: 'audio/mp4; codecs="opus"', codec: ['-c:a', 'libopus', '-b:a', '160k', '-ac', '2'] },
];

/** The first royalty-free output MSE can append here, or null if none (e.g. Safari). */
function pickOutput() {
  const MS = typeof MediaSource !== 'undefined' ? MediaSource : null;
  return OUTPUTS.find((o) => MS && MS.isTypeSupported(o.mime)) || null;
}

// The MSE mimes the transcoder may produce — exported so the main chunk can gate transcoding
// on MSE support WITHOUT importing this (@ffmpeg-pulling) module. Kept in sync with OUTPUTS.
export const TRANSCODE_OUT_MIMES = OUTPUTS.map((o) => o.mime);

// Emscripten MEMFS never shrinks; rebuild the instance periodically to reclaim its heap
// over a long movie (each window writes+reads ~hundreds of KB).
const RECREATE_EVERY = 40;

export class AudioTranscoder {
  constructor({ coreURL, wasmURL, onLog } = {}) {
    if (!coreURL || !wasmURL) {
      throw new Error('AudioTranscoder requires ffmpeg coreURL and wasmURL');
    }
    this.coreURL = coreURL;
    this.wasmURL = wasmURL;
    // Choose the royalty-free output codec this browser can actually append via MSE.
    this.output = pickOutput();
    if (!this.output) {
      throw new Error('AudioTranscoder: no royalty-free MSE output (Opus/FLAC) supported here');
    }
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
    return this.output.mime;
  }

  async ensureLoaded() {
    if (this.ffmpeg) return this.ffmpeg;
    if (!this.loadPromise) {
      this.loadPromise = (async () => {
        const ff = new FFmpeg();
        if (this.onLog) ff.on('log', ({ message }) => this.onLog(message));
        await ff.load({
          coreURL: await toBlobURL(this.coreURL, 'text/javascript'),
          wasmURL: await toBlobURL(this.wasmURL, 'application/wasm'),
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
      await ff.exec(['-i', input, '-map', '0:a:0', ...this.output.codec, ...MUX_ARGS, OUTPUT]);
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
