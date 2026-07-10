// ASS/SSA rendering via libass (JASSUB) for subtitle tracks muxed into the MKV.
//
// We never have the whole .ass file: the script *header* (styles, resolution) comes
// from the track's CodecPrivate, and the dialogue events are streamed in window-by-
// window by the MseController as it buffers clusters (see SubtitleFeeder in mse.js).
// Each ASS line carries its own absolute Start/End, so feed order is irrelevant — we
// just accumulate the events seen so far (deduped by their MKV ReadOrder). libass
// resolves styles by name, which is why we feed text rather than libass's index-based
// createEvent.
//
// Two responsibilities are split apart so multiple tracks can be shown at once:
//   • SubtitleTrack   — the growing cue cache for ONE track. Fed continuously for every
//                       ASS track from playback start (whether or not it's displayed), so
//                       enabling a track is instant and never drops the line already on
//                       screen. No JASSUB — just accumulated `Dialogue:` lines + header.
//   • SubtitleRenderer — one JASSUB instance + canvas overlay, created only for a track
//                        that's actually displayed (max two at a time). Seeded from the
//                        cache's full document on show, then updated (debounced) as more
//                        cues arrive.
//
// JASSUB resolves its worker/wasm/default-font via `new URL(…, import.meta.url)`; we let
// Vite handle that (jassub is in optimizeDeps.exclude) rather than passing workerUrl/wasmUrl.
import JASSUB from 'jassub';

// Bitmap subtitles (PGS / S_HDMV/PGS) are rendered by libpgs. Same split as ASS:
// BitmapSubtitleTrack is the growing `.sup` cue cache (fed continuously per track),
// BitmapSubtitleRenderer wraps a libpgs PgsRenderer over a canvas overlay for a
// displayed track. See the end of this file. libpgs is a plain ESM module rendered
// on the main thread (mode below), so — unlike jassub — it needs no worker/wasm
// plumbing and bundles normally.
import { PgsRenderer } from 'libpgs';

const DEFAULT_HEADER =
  '[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\n' +
  '[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n' +
  'Style: Default,Arial,72,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,0,2,10,10,30,1\n\n';

const EVENTS_FORMAT =
  '[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n';

const REBUILD_DEBOUNCE_MS = 250;

const pad2 = (n) => String(n).padStart(2, '0');

/** Milliseconds → ASS timestamp `H:MM:SS.cc` (centiseconds). */
function fmtAssTime(ms) {
  const cs = Math.max(0, Math.round(ms / 10));
  const h = Math.floor(cs / 360000);
  const m = Math.floor((cs % 360000) / 6000);
  const s = Math.floor((cs % 6000) / 100);
  const c = cs % 100;
  return `${h}:${pad2(m)}:${pad2(s)}.${pad2(c)}`;
}

// An MKV ASS block payload is `ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,
// Effect,Text` with no timestamps. Turn it into a `Dialogue:` line, injecting the
// Start/End we recovered from the block timecode + duration. Text may contain commas,
// so only the first 8 fields are split off.
function toDialogue(startMs, endMs, rawText) {
  const parts = rawText.split(',');
  if (parts.length < 9) return null; // not a well-formed ASS event line
  const layer = parts[1] || '0';
  const rest = parts.slice(2).join(','); // Style,Name,ML,MR,MV,Effect,Text
  return `Dialogue: ${layer},${fmtAssTime(startMs)},${fmtAssTime(endMs)},${rest}`;
}

/** The MKV ReadOrder (first field) — the stable dedup key across seeks. */
function readOrderOf(rawText) {
  const i = rawText.indexOf(',');
  return i === -1 ? rawText : rawText.slice(0, i);
}

function normalizeHeader(header) {
  let h = header && header.trim() ? header : DEFAULT_HEADER;
  if (!h.endsWith('\n')) h += '\n';
  // Some CodecPrivate blobs omit the [Events] Format line; libass needs it to parse
  // the Dialogue lines we append.
  if (!/\[Events\]/i.test(h)) h += '\n' + EVENTS_FORMAT;
  return h;
}

/**
 * The cue cache for a single subtitle track. Fed continuously by a SubtitleFeeder
 * (mse.js) for the whole session, independent of whether the track is displayed.
 * Notifies `onChange` when new cues land so an attached SubtitleRenderer can re-render.
 */
export class SubtitleTrack {
  /** @param header the track's CodecPrivate (ASS script header). */
  constructor(header) {
    this.header = normalizeHeader(header);
    this.events = new Map(); // ReadOrder → Dialogue line
    this.onChange = null; // set by the manager while this track is displayed
  }

  /** Merge a batch of `{start, end, text}` cues (from subtitle_events). */
  addEvents(cues) {
    if (!Array.isArray(cues) || cues.length === 0) return;
    let added = 0;
    for (const c of cues) {
      const key = readOrderOf(c.text);
      if (this.events.has(key)) continue;
      const line = toDialogue(c.start, c.end, c.text);
      if (!line) continue;
      this.events.set(key, line);
      added++;
    }
    if (added && this.onChange) this.onChange();
  }

  /** The full ASS document (header + every accumulated event). */
  buildDoc() {
    return this.header + [...this.events.values()].join('\n') + '\n';
  }
}

/**
 * One JASSUB instance rendering one displayed subtitle track over its own canvas overlay.
 * Created only when a track is shown; several can coexist (dual subtitles), each with its
 * own canvas and worker. All render at the track's native ASS position.
 */
export class SubtitleRenderer {
  /**
   * @param video HTMLVideoElement (drives timing/resize via requestVideoFrameCallback)
   * @param fonts Uint8Array[] font attachments to seed the instance with
   *
   * We create a fresh canvas (JASSUB transfers it to an OffscreenCanvas, so it can't be
   * reused on reload) and mount it right after the <video> inside media-container. DOM
   * order puts it above the video but below media-controls (which comes later in the
   * markup), so the overlay sits under the control bar. JASSUB sizes/positions it to
   * match the video (they share media-container as their containing block). Multiple
   * overlays are transparent and stack cleanly.
   */
  constructor(video, fonts = []) {
    this.video = video;
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'subtitle-overlay'; // JASSUB sets size/position; CSS sets the rest
    this.canvas.style.display = 'none';
    video.insertAdjacentElement('afterend', this.canvas);
    this.instance = null;
    this.fonts = fonts.slice(); // Uint8Array[] gathered from attachments
    this.doc = null; // most recent document handed to buildDoc/show/update
    this.rebuildTimer = null;
  }

  ensureInstance() {
    if (this.instance) return;
    this.instance = new JASSUB({
      video: this.video,
      canvas: this.canvas,
      subContent: this.doc || '',
      fonts: this.fonts.slice(),
    });
  }

  // The subtitle/font methods (setTrack, addFont, freeTrack, …) live on the worker proxy
  // `instance.renderer`, which only exists after `instance.ready` resolves. Run `fn` with it.
  async _withRenderer(fn) {
    if (!this.instance) return;
    try {
      await this.instance.ready;
    } catch (_) {
      return;
    }
    const r = this.instance && this.instance.renderer;
    if (r) {
      try {
        await fn(r);
      } catch (e) {
        console.warn('jassub renderer call failed', e);
      }
    }
  }

  /** Start rendering `doc` (a full ASS document) immediately. */
  async show(doc) {
    this.doc = doc;
    this.ensureInstance();
    if (this.canvas) this.canvas.style.display = '';
    await this._withRenderer((r) => r.setTrack(this.doc));
  }

  /** Re-render with a fresh `doc`, debounced (cues trickle in as playback streams). */
  update(doc) {
    this.doc = doc;
    if (this.rebuildTimer || !this.instance) return;
    this.rebuildTimer = setTimeout(() => {
      this.rebuildTimer = null;
      this._withRenderer((r) => r.setTrack(this.doc));
    }, REBUILD_DEBOUNCE_MS);
  }

  /** Add a font (Uint8Array) from an attachment. Works before or after the instance exists. */
  addFont(data) {
    this.fonts.push(data); // also picked up by the `fonts` option if the instance is created later
    if (this.instance) {
      this._withRenderer((r) => r.addFonts([data])).then(() => {
        if (this.doc) this.update(this.doc); // re-resolve glyphs against the new font
      });
    }
  }

  destroy() {
    if (this.rebuildTimer) clearTimeout(this.rebuildTimer);
    this.rebuildTimer = null;
    if (this.instance) {
      try {
        this.instance.destroy(); // also removes the (transferred) canvas from the DOM
      } catch (_) {}
      this.instance = null;
    } else if (this.canvas) {
      this.canvas.remove();
    }
    this.canvas = null;
    this.doc = null;
    this.fonts = [];
  }
}

/** base64 (as delivered by subtitle_bitmap_events) → Uint8Array. */
function b64ToBytes(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/**
 * The `.sup` cue cache for a single PGS (bitmap) subtitle track — the bitmap analogue of
 * {@link SubtitleTrack}. Fed continuously by a SubtitleFeeder (mse.js): each event is one
 * PGS display set, already reconstructed into a `.sup` fragment by the WASM core and
 * base64-encoded. We keep them keyed by presentation timestamp (the stable dedup key across
 * re-fetched/overlapping windows) and concatenate them, ascending, into the growing `.sup`
 * buffer handed to libpgs. Notifies `onChange` when new display sets land.
 */
export class BitmapSubtitleTrack {
  constructor() {
    this.events = new Map(); // pts (ms) → Uint8Array (.sup fragment)
    this.onChange = null; // set by the manager while this track is displayed
  }

  /** Merge a batch of `{pts, sup}` display sets (from subtitle_bitmap_events). */
  addEvents(cues) {
    if (!Array.isArray(cues) || cues.length === 0) return;
    let added = 0;
    for (const c of cues) {
      if (this.events.has(c.pts)) continue;
      this.events.set(c.pts, b64ToBytes(c.sup));
      added++;
    }
    if (added && this.onChange) this.onChange();
  }

  /** The whole `.sup` bitstream so far (fragments concatenated in ascending-pts order). */
  buildBuffer() {
    const keys = [...this.events.keys()].sort((a, b) => a - b);
    let total = 0;
    for (const k of keys) total += this.events.get(k).length;
    const out = new Uint8Array(total);
    let off = 0;
    for (const k of keys) {
      const frag = this.events.get(k);
      out.set(frag, off);
      off += frag.length;
    }
    return out.buffer;
  }
}

/**
 * One libpgs {@link PgsRenderer} rendering one displayed PGS track over its own canvas
 * overlay — the bitmap analogue of {@link SubtitleRenderer}. libpgs syncs to the <video>'s
 * timeupdate itself and re-renders on load (`onTimestampsUpdated`), so we only feed it the
 * `.sup` buffer: `show()` seeds it instantly from the cache, `update()` re-feeds (debounced)
 * as more display sets stream in. Runs on the main thread (no worker/wasm to bundle) — PGS
 * updates are infrequent, so decode cost is negligible.
 */
export class BitmapSubtitleRenderer {
  /** @param video HTMLVideoElement (libpgs reads currentTime and listens for timeupdate). */
  constructor(video) {
    this.video = video;
    // Our own canvas (not libpgs's auto-created one) so it mounts right after the <video>,
    // stacking under media-controls exactly like the JASSUB overlay. libpgs won't touch its
    // position/DOM when we pass it in; the CSS class letterboxes it over the video.
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'subtitle-overlay subtitle-overlay--pgs';
    this.canvas.style.display = 'none';
    video.insertAdjacentElement('afterend', this.canvas);
    this.instance = null;
    this.buffer = null; // most recent `.sup` ArrayBuffer handed to show/update
    this.rebuildTimer = null;
  }

  ensureInstance() {
    if (this.instance) return;
    this.instance = new PgsRenderer({
      video: this.video,
      canvas: this.canvas,
      mode: 'mainThread',
      aspectRatio: 'contain', // matches the video's object-fit; letterboxes the composition
    });
  }

  /** Start rendering `buffer` (a full `.sup` bitstream) immediately. */
  show(buffer) {
    this.buffer = buffer;
    this.ensureInstance();
    if (this.canvas) this.canvas.style.display = '';
    if (buffer && buffer.byteLength) this.instance.loadFromBuffer(buffer);
  }

  /** Re-feed a grown `.sup` buffer, debounced (display sets trickle in as playback streams). */
  update(buffer) {
    this.buffer = buffer;
    if (this.rebuildTimer || !this.instance) return;
    this.rebuildTimer = setTimeout(() => {
      this.rebuildTimer = null;
      if (this.buffer && this.buffer.byteLength) this.instance.loadFromBuffer(this.buffer);
    }, REBUILD_DEBOUNCE_MS);
  }

  destroy() {
    if (this.rebuildTimer) clearTimeout(this.rebuildTimer);
    this.rebuildTimer = null;
    if (this.instance) {
      try {
        this.instance.dispose(); // detaches the timeupdate listener; leaves our canvas (external)
      } catch (_) {}
      this.instance = null;
    }
    if (this.canvas) this.canvas.remove();
    this.canvas = null;
    this.buffer = null;
  }
}
