// Drives Media Source Extensions from a WASM MatroskaPlayer: one SourceBuffer for
// video and one for audio, each fed fMP4 segments tiled on cue boundaries. Audio is
// switchable (changeType + re-feed). Seeking re-feeds from the cue nearest the target.

const BIG = (n) => BigInt(Math.max(0, Math.round(n)));

// Holes up to this size (ms) are treated as continuous buffer. Sized to swallow the
// per-fragment gaps left by self-contained transcoded AAC (encoder priming, tens of
// ms) without masking a genuinely missing segment. Browsers play across gaps this
// small, so bridging them only affects our buffer-fullness accounting.
const GAP_TOLERANCE_MS = 500;

// Cap on segments queued-but-not-yet-appended per stream during one fill pass. Stops a
// slow producer (e.g. ffmpeg transcode) from running far ahead of the SourceBuffer and
// monopolizing the shared topUp pass.
const MAX_PENDING = 4;

// --- DEBUG instrumentation -------------------------------------------------
// Toggle at runtime in the console: `window.MSE_DEBUG = true` (or `false`).
// Defaults to on so the buffering bug can be observed without a rebuild.
const DEBUG = () =>
  typeof window === 'undefined' ? false : window.MSE_DEBUG !== false;
const dlog = (...a) => {
  if (DEBUG()) console.log('[mse]', ...a);
};

// Buffered ranges of a SourceBuffer as [[start,end],…] in seconds (3dp).
function describeBuffered(sb) {
  const r = sb.buffered;
  const out = [];
  for (let i = 0; i < r.length; i++) {
    out.push([+r.start(i).toFixed(3), +r.end(i).toFixed(3)]);
  }
  return out;
}

// Holes between adjacent buffered ranges (seconds). A non-empty result is the
// smoking gun for the stall: the playhead can be trapped before one of these.
function bufferedGaps(sb) {
  const r = sb.buffered;
  const gaps = [];
  for (let i = 1; i < r.length; i++) {
    gaps.push({
      from: +r.end(i - 1).toFixed(3),
      to: +r.start(i).toFixed(3),
      size: +(r.start(i) - r.end(i - 1)).toFixed(3),
    });
  }
  return gaps;
}

// Whether `t` (seconds) falls inside some buffered range, and that range's end.
// When `inRange` is false the playhead is sitting in a gap (or before the first
// range) — the exact condition that makes topUp() over-download.
function playheadState(sb, t) {
  const r = sb.buffered;
  for (let i = 0; i < r.length; i++) {
    if (t >= r.start(i) - 0.1 && t < r.end(i)) {
      return { inRange: true, end: +r.end(i).toFixed(3) };
    }
  }
  return { inRange: false, end: null };
}

/** Serializes appends/removes on a SourceBuffer through its `updateend` event. */
class Pump {
  constructor(sb) {
    this.sb = sb;
    this.queue = [];
    sb.addEventListener('updateend', () => this.flush());
  }
  // `timestampOffset` (seconds) is applied just before the append when set — used by the
  // transcoding path to place each self-contained, zero-anchored fragment on the timeline.
  push(buf, timestampOffset = null) {
    this.queue.push({ buf, timestampOffset });
    this.flush();
  }
  flush() {
    if (this.sb.updating || this.queue.length === 0) return;
    const { buf, timestampOffset } = this.queue.shift();
    try {
      if (timestampOffset != null && this.sb.timestampOffset !== timestampOffset) {
        this.sb.timestampOffset = timestampOffset;
      }
      this.sb.appendBuffer(buf);
    } catch (e) {
      console.error('appendBuffer failed', e);
    }
  }
  clear() {
    this.queue.length = 0;
  }
}

/** Produces and appends media segments for one track, tiled on `boundaries` (ms). */
class TrackFeeder {
  constructor(player, trackNumber, boundaries, pump) {
    this.player = player;
    this.trackNumber = trackNumber;
    this.boundaries = boundaries; // ascending ms, last entry = duration
    this.pump = pump;
    this.index = 0;
    this.busy = false;
    this.done = false;
    this.generation = 0; // bumped on seek to discard in-flight segments
    this.fetches = 0; // DEBUG: total media_segment requests issued
  }

  /** Reset feeding to the boundary at or before `timeMs`. */
  seekTo(timeMs) {
    let idx = 0;
    for (let i = 0; i < this.boundaries.length - 1; i++) {
      if (this.boundaries[i] <= timeMs) idx = i;
      else break;
    }
    this.index = idx;
    this.done = false;
    this.generation += 1;
  }

  async feedOne() {
    if (this.busy || this.done) return;
    if (this.index >= this.boundaries.length - 1) {
      this.done = true;
      return;
    }
    this.busy = true;
    const gen = this.generation;
    const start = this.boundaries[this.index];
    const end = this.boundaries[this.index + 1];
    this.index += 1;
    this.fetches += 1;
    try {
      const seg = await this.player.media_segment(BIG(this.trackNumber), BIG(start), BIG(end));
      // A seek may have happened during the await; drop the now-stale segment.
      if (gen === this.generation && seg && seg.length) this.pump.push(seg);
      dlog('feed track', this.trackNumber, 'window', [start, end],
        'bytes', seg ? seg.length : 0, 'fetches', this.fetches);
    } catch (e) {
      console.error('media_segment failed', e);
    }
    this.busy = false;
  }
}

/**
 * Like {@link TrackFeeder}, but for an audio track whose codec the browser can't decode
 * natively: each window is fetched from the WASM core as a self-contained Matroska chunk
 * (`audio_chunk`), transcoded to fragmented Opus-in-MP4 by ffmpeg.wasm, and appended at the
 * chunk's true start time via the pump's per-segment `timestampOffset`. Each fragment is
 * self-contained (its own init), so no separate init segment is fed.
 */
class TranscodingTrackFeeder {
  constructor(player, trackNumber, boundaries, pump, transcoder) {
    this.player = player;
    this.trackNumber = trackNumber;
    this.boundaries = boundaries;
    this.pump = pump;
    this.transcoder = transcoder;
    this.index = 0;
    this.busy = false;
    this.done = false;
    this.generation = 0;
    this.fetches = 0; // DEBUG: total audio_chunk requests issued
  }

  seekTo(timeMs) {
    let idx = 0;
    for (let i = 0; i < this.boundaries.length - 1; i++) {
      if (this.boundaries[i] <= timeMs) idx = i;
      else break;
    }
    this.index = idx;
    this.done = false;
    this.generation += 1;
  }

  async feedOne() {
    if (this.busy || this.done) return;
    if (this.index >= this.boundaries.length - 1) {
      this.done = true;
      return;
    }
    this.busy = true;
    const gen = this.generation;
    const start = this.boundaries[this.index];
    const end = this.boundaries[this.index + 1];
    this.index += 1;
    this.fetches += 1;
    try {
      const chunk = await this.player.audio_chunk(BIG(this.trackNumber), BIG(start), BIG(end));
      if (chunk) {
        const base = chunk.base_seconds;
        const mkv = chunk.data; // getter moves the bytes out of the wasm struct
        chunk.free();
        // A seek during the wasm read makes this window stale — skip the costly transcode.
        if (gen === this.generation) {
          const frag = await this.transcoder.transcode(mkv);
          if (gen === this.generation && frag && frag.length) this.pump.push(frag, base);
        }
      }
    } catch (e) {
      console.error('audio_chunk/transcode failed', e);
    }
    this.busy = false;
  }
}

/**
 * Feeds streamed subtitle cues for one ASS track, tiled on the same `boundaries` as
 * video/audio. Each window's cues come from clusters the video pass has usually already
 * fetched (cache hits), so this rides along the existing single forward stream. Events
 * persist in the track's SubtitleTrack cache, so backward seeks need no re-feed; forward
 * seeks re-tile. One feeder runs per ASS track for the whole session.
 */
class SubtitleFeeder {
  constructor(player, trackNumber, boundaries, sink) {
    this.player = player;
    this.trackNumber = trackNumber;
    this.boundaries = boundaries;
    this.sink = sink; // SubtitleTrack (cue cache)
    this.index = 0;
    this.busy = false;
    this.done = false;
    this.generation = 0;
  }

  /** Time (ms) through which windows have been requested — the next unfed window start. */
  fedThroughMs() {
    return this.index >= this.boundaries.length - 1
      ? Infinity
      : this.boundaries[this.index];
  }

  seekTo(timeMs) {
    let idx = 0;
    for (let i = 0; i < this.boundaries.length - 1; i++) {
      if (this.boundaries[i] <= timeMs) idx = i;
      else break;
    }
    this.index = idx;
    this.done = false;
    this.generation += 1;
  }

  async feedOne() {
    if (this.busy || this.done) return;
    if (this.index >= this.boundaries.length - 1) {
      this.done = true;
      return;
    }
    this.busy = true;
    const gen = this.generation;
    const start = this.boundaries[this.index];
    const end = this.boundaries[this.index + 1];
    this.index += 1;
    try {
      const json = await this.player.subtitle_events(BIG(this.trackNumber), BIG(start), BIG(end));
      if (gen === this.generation && json) this.sink.addEvents(JSON.parse(json));
    } catch (e) {
      console.error('subtitle_events failed', e);
    }
    this.busy = false;
  }
}

export class MseController {
  /**
   * @param player  MatroskaPlayer (wasm)
   * @param video   HTMLVideoElement
   * @param tracks  parsed track list (from player.tracks())
   * @param durationMs total duration
   * @param cueTimes  array of cue boundary times in ms (may be empty)
   */
  constructor(player, video, tracks, durationMs, cueTimes, transcoder = null) {
    this.player = player;
    this.video = video;
    this.tracks = tracks;
    this.durationMs = durationMs;
    this.boundaries = buildBoundaries(cueTimes, durationMs);
    // Optional ffmpeg.wasm AudioTranscoder for codecs MSE can't decode natively (null
    // when the feature is disabled or no transcodable track is selected).
    this.transcoder = transcoder;

    this.mediaSource = null;
    this.video$ = null; // { feeder, pump, track }
    this.audio$ = null;
    // One SubtitleFeeder per ASS track, each filling that track's SubtitleTrack cache
    // continuously from playback start (regardless of which tracks are displayed).
    this.subFeeders = [];
    this.bufferAheadMs = 5000;
    // Transcoded audio is far slower to produce than a byte-copy remux, so buffer it
    // further ahead to stay clear of the playhead.
    this.transcodeBufferAheadMs = 20000;
    // Smaller buffer required before playback can begin — keeps startup latency (and the
    // up-front download) low, while steady-state still fills to bufferAheadMs.
    this.startBufferMs = 1500;
    this.topUpQueued = false;
  }

  async start(videoTrack, audioTrack) {
    // DEBUG: reachable from the console as `window.__mse.debugReport()`.
    if (typeof window !== 'undefined') window.__mse = this;
    this.mediaSource = new MediaSource();
    this.video.src = URL.createObjectURL(this.mediaSource);

    await new Promise((resolve) => {
      this.mediaSource.addEventListener('sourceopen', resolve, { once: true });
    });

    if (Number.isFinite(this.durationMs) && this.durationMs > 0) {
      this.mediaSource.duration = this.durationMs / 1000;
    }

    if (videoTrack) this.video$ = this.setupTrack(videoTrack);
    if (audioTrack) this.audio$ = this.setupTrack(audioTrack);

    this.video.addEventListener('timeupdate', () => this.topUp());
    this.video.addEventListener('seeking', () => this.onSeek());
    this.video.addEventListener('waiting', () => this.topUp());

    // DEBUG: when the element stalls, report exactly where the playhead is
    // relative to the buffered ranges — this distinguishes "trapped in a gap"
    // (the bug) from a genuine starved buffer.
    this.video.addEventListener('waiting', () => {
      if (DEBUG()) {
        console.warn('[mse] WAITING (stall) at', +this.video.currentTime.toFixed(3) + 's');
        this.debugReport();
      }
    });
    this.video.addEventListener('stalled', () => {
      if (DEBUG()) this.debugReport('stalled');
    });

    await this.topUp(this.startBufferMs);
  }

  // DEBUG: dump each stream's buffered ranges, gaps, playhead position, and the
  // number of segment fetches issued so far. Call from the console as
  // `controller.debugReport()` while playback is stuck.
  debugReport(tag = 'report') {
    const t = this.video.currentTime;
    for (const stream of [this.video$, this.audio$]) {
      if (!stream) continue;
      const label = stream === this.video$ ? 'video' : 'audio';
      const ph = playheadState(stream.sb, t);
      const gaps = bufferedGaps(stream.sb);
      const gapSizes = gaps.map((g) => g.size);
      // What this stream's topUp loop is *trying* to reach vs. what it can
      // actually see at the playhead — a large shortfall with feederDone:false
      // is the over-download condition.
      const ahead =
        stream.transcoded ? Math.max(this.bufferAheadMs, this.transcodeBufferAheadMs) : this.bufferAheadMs;
      console.log(`[mse] ${tag} ${label}`, {
        transcoded: stream.transcoded,
        currentTime: +t.toFixed(3),
        playheadInRange: ph.inRange,
        rangeEndS: ph.end,
        targetS: +(t + ahead / 1000).toFixed(3),
        shortfallS: ph.end != null ? +(t + ahead / 1000 - ph.end).toFixed(3) : null,
        rangeCount: stream.sb.buffered.length,
        gapCount: gaps.length,
        gapTotalS: +gapSizes.reduce((a, b) => a + b, 0).toFixed(3),
        gapMinS: gapSizes.length ? Math.min(...gapSizes) : null,
        gapMaxS: gapSizes.length ? Math.max(...gapSizes) : null,
        firstGaps: gaps.slice(0, 5),
        fetches: stream.feeder.fetches,
        feederIndex: stream.feeder.index,
        feederDone: stream.feeder.done,
        pumpQueue: stream.pump.queue.length,
        msReadyState: this.mediaSource && this.mediaSource.readyState,
      });
    }
  }

  // Resolve how an audio track will be played: natively (its own mime) or via transcoding
  // (the transcoder's output mime). Returns null if it can't be played at all.
  audioPlan(track) {
    if (track.mime && MediaSource.isTypeSupported(track.mime)) {
      return { mime: track.mime, transcoded: false };
    }
    if (
      track.type === 'audio' &&
      this.transcoder &&
      MediaSource.isTypeSupported(this.transcoder.mime)
    ) {
      return { mime: this.transcoder.mime, transcoded: true };
    }
    return null;
  }

  setupTrack(track) {
    // Video (and natively-playable audio) take the direct remux path.
    if (track.type !== 'audio') {
      if (!MediaSource.isTypeSupported(track.mime)) {
        console.warn(`unsupported: track ${track.number} ${track.mime}`);
        return null;
      }
      const sb = this.mediaSource.addSourceBuffer(track.mime);
      const pump = new Pump(sb);
      const init = this.player.init_segment(BIG(track.number));
      if (init && init.length) pump.push(init);
      const feeder = new TrackFeeder(this.player, track.number, this.boundaries, pump);
      return { track, sb, pump, feeder, mime: track.mime, transcoded: false };
    }

    const plan = this.audioPlan(track);
    if (!plan) {
      console.warn(`unsupported: track ${track.number} ${track.mime || track.codec_id}`);
      return null;
    }
    const sb = this.mediaSource.addSourceBuffer(plan.mime);
    const pump = new Pump(sb);
    let feeder;
    if (plan.transcoded) {
      // Each transcoded fragment carries its own init — no separate init segment.
      feeder = new TranscodingTrackFeeder(this.player, track.number, this.boundaries, pump, this.transcoder);
    } else {
      const init = this.player.init_segment(BIG(track.number));
      if (init && init.length) pump.push(init);
      feeder = new TrackFeeder(this.player, track.number, this.boundaries, pump);
    }
    return { track, sb, pump, feeder, mime: plan.mime, transcoded: plan.transcoded };
  }

  // End (ms) of the buffered run that contains `currentMs`, treating holes smaller
  // than GAP_TOLERANCE_MS as part of the run. Transcoded AAC fragments don't abut
  // (each carries ~tens of ms of encoder-priming gap), so a strict "single range"
  // measure reports the buffer as perpetually short of target — the feeder then
  // transcodes without end (and, sharing the single topUp pass, starves video).
  // Browsers already play across these sub-frame holes, so bridging them here makes
  // the fill loop terminate. If the playhead is in a real gap (e.g. just after a
  // backward seek), returns `currentMs` so it reads as "below target" and feeds.
  bufferedEndMs(stream, currentMs) {
    return bufferedEndAcrossGaps(stream.sb.buffered, currentMs, GAP_TOLERANCE_MS);
  }

  // Buffer-ahead target (ms) for a stream. Transcoded audio is built much further
  // ahead in steady state (it's slow to produce), but not during the tiny initial
  // fill (aheadMs <= startBufferMs) so playback still starts promptly.
  targetAheadMs(stream, aheadMs) {
    return stream.transcoded && aheadMs > this.startBufferMs
      ? Math.max(aheadMs, this.transcodeBufferAheadMs)
      : aheadMs;
  }

  // How far below its target a stream's buffer is at the playhead (ms; <=0 = full).
  deficitMs(stream, currentMs, aheadMs) {
    return currentMs + this.targetAheadMs(stream, aheadMs) - this.bufferedEndMs(stream, currentMs);
  }

  // Fill the buffers with a SINGLE interleaved loop rather than draining each stream
  // in turn. The source allows only one in-flight read (see stream_source.rs), so we
  // still feed one segment at a time — but after every segment we re-read the playhead
  // and feed whichever stream is now neediest, with video prioritized. This stops a
  // slow ffmpeg audio transcode (which fills 20s ahead) from monopolizing the pass and
  // starving the much shallower (5s) video buffer, which was the real stall.
  async topUp(aheadMs = this.bufferAheadMs) {
    if (this.topUpQueued) return;
    this.topUpQueued = true;
    try {
      let guard = 0;
      while (guard++ < 256) {
        const currentMs = this.video.currentTime * 1000;
        const candidates = [this.video$, this.audio$]
          .filter(Boolean)
          .map((stream) => ({
            stream,
            prioritized: stream === this.video$,
            deficit: this.deficitMs(stream, currentMs, aheadMs),
            pumpFull: stream.pump.queue.length >= MAX_PENDING,
            done: stream.feeder.done,
          }));
        const pick = chooseFeedTarget(candidates);
        if (!pick) break; // both at target (or backpressured) — re-triggered on timeupdate
        await pick.stream.feeder.feedOne();
      }
      if (DEBUG() && guard >= 256) {
        console.warn('[mse] topUp hit guard cap (256 feeds in one pass)');
      }
      // Subtitles ride their own sequential chain (cues are tiny). Fetch a bit further
      // ahead than media; this never blocks the media loop above (runs after it).
      if (this.subFeeders.length) {
        const subTarget = this.video.currentTime * 1000 + aheadMs + 4000;
        for (const feeder of this.subFeeders) {
          let sguard = 0;
          while (!feeder.done && feeder.fedThroughMs() < subTarget && sguard++ < 128) {
            await feeder.feedOne();
          }
        }
      }
      this.maybeEndOfStream();
    } finally {
      this.topUpQueued = false;
    }
  }

  maybeEndOfStream() {
    const streams = [this.video$, this.audio$].filter(Boolean);
    if (
      streams.length &&
      streams.every((s) => s.feeder.done && s.pump.queue.length === 0 && !s.sb.updating) &&
      this.mediaSource.readyState === 'open'
    ) {
      try {
        this.mediaSource.endOfStream();
      } catch (_) {}
    }
  }

  onSeek() {
    const t = this.video.currentTime * 1000;
    for (const stream of [this.video$, this.audio$]) {
      if (!stream) continue;
      stream.pump.clear();
      stream.feeder.seekTo(t);
    }
    for (const feeder of this.subFeeders) feeder.seekTo(t);
    this.topUp();
  }

  /**
   * Register the cue caches to stream into — one entry `{ trackNumber, sink }` per ASS
   * track (sink is its SubtitleTrack). Feeders run for every track from now on so any
   * track can be displayed instantly from its cache; which tracks actually render is the
   * player's concern, not ours.
   */
  setSubtitleSinks(entries) {
    const t = this.video.currentTime * 1000;
    this.subFeeders = entries.map(({ trackNumber, sink }) => {
      const feeder = new SubtitleFeeder(this.player, trackNumber, this.boundaries, sink);
      feeder.seekTo(t);
      return feeder;
    });
    this.topUp();
  }

  /** Switch the active audio track, rebuilding the audio buffer at the playhead. */
  async switchAudio(trackNumber) {
    const track = this.tracks.find((t) => t.number === trackNumber);
    if (!track || !this.audio$) return;
    const plan = this.audioPlan(track);
    if (!plan) return; // not playable (e.g. transcoding disabled)
    const stream = this.audio$;

    // Drain anything queued and wait for the buffer to be idle.
    stream.pump.clear();
    await this.untilIdle(stream.sb);

    // Switch codec/container if needed, then clear the old audio and re-feed from the playhead.
    if (plan.mime !== stream.mime && typeof stream.sb.changeType === 'function') {
      stream.sb.changeType(plan.mime);
      stream.mime = plan.mime;
    }
    if (stream.sb.buffered.length) {
      stream.sb.remove(0, Infinity);
      await this.untilIdle(stream.sb);
    }

    stream.track = track;
    stream.transcoded = plan.transcoded;
    if (plan.transcoded) {
      stream.feeder = new TranscodingTrackFeeder(this.player, trackNumber, this.boundaries, stream.pump, this.transcoder);
    } else {
      stream.feeder = new TrackFeeder(this.player, trackNumber, this.boundaries, stream.pump);
      const init = this.player.init_segment(BIG(trackNumber));
      if (init && init.length) stream.pump.push(init);
    }
    stream.feeder.seekTo(this.video.currentTime * 1000);
    await this.topUp();
  }

  untilIdle(sb) {
    if (!sb.updating) return Promise.resolve();
    return new Promise((resolve) => sb.addEventListener('updateend', resolve, { once: true }));
  }
}

/**
 * Choose which stream to feed next in the interleaved fill loop. Skips streams that
 * are done, backpressured (`pumpFull`), or already at target (`deficit <= 0`). Among
 * the rest, a `prioritized` stream (video) always wins over a non-prioritized one
 * (transcoded audio) regardless of relative deficit, so video — cheap to read and
 * shallowly buffered — is kept ahead of the playhead. Ties broken by larger deficit.
 * Returns the chosen candidate object, or null if none need feeding. Pure/testable.
 */
export function chooseFeedTarget(candidates) {
  let best = null;
  let bestScore = -Infinity;
  for (const c of candidates) {
    if (c.done || c.pumpFull || c.deficit <= 0) continue;
    const score = c.deficit + (c.prioritized ? 1e12 : 0);
    if (score > bestScore) {
      bestScore = score;
      best = c;
    }
  }
  return best;
}

/**
 * End (ms) of the buffered run containing `currentMs`, bridging holes up to `tolMs`.
 * Pure and DOM-free (takes any `{length, start(i), end(i)}` in seconds, like a
 * `TimeRanges`) so it can be unit-tested. Returns `currentMs` when the playhead is
 * not inside (within 100ms of) any run — i.e. it sits in a real gap.
 */
export function bufferedEndAcrossGaps(buffered, currentMs, tolMs) {
  let end = null;
  for (let i = 0; i < buffered.length; i++) {
    const start = buffered.start(i) * 1000;
    const e = buffered.end(i) * 1000;
    if (end === null) {
      if (currentMs >= start - 100 && currentMs < e) end = e;
    } else if (start - end <= tolMs) {
      end = e; // bridge a small hole and keep extending the run
    } else {
      break; // a real gap — the run ends here
    }
  }
  return end === null ? currentMs : end;
}

function buildBoundaries(cueTimes, durationMs) {
  let bounds = Array.isArray(cueTimes) ? cueTimes.slice().sort((a, b) => a - b) : [];
  if (bounds.length < 2) {
    // No usable cues — tile in fixed 4s windows.
    bounds = [];
    const step = 4000;
    const end = durationMs > 0 ? durationMs : 10 * 60 * 1000;
    for (let t = 0; t < end; t += step) bounds.push(t);
  }
  if (bounds[0] !== 0) bounds.unshift(0);
  const last = durationMs > 0 ? durationMs : bounds[bounds.length - 1] + 4000;
  if (bounds[bounds.length - 1] < last) bounds.push(last);
  return bounds;
}
