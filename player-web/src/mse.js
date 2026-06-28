// Drives Media Source Extensions from a WASM MatroskaPlayer: one SourceBuffer for
// video and one for audio, each fed fMP4 segments tiled on cue boundaries. Audio is
// switchable (changeType + re-feed). Seeking re-feeds from the cue nearest the target.

const BIG = (n) => BigInt(Math.max(0, Math.round(n)));

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
    try {
      const seg = await this.player.media_segment(BIG(this.trackNumber), BIG(start), BIG(end));
      // A seek may have happened during the await; drop the now-stale segment.
      if (gen === this.generation && seg && seg.length) this.pump.push(seg);
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
 * persist in libass, so backward seeks need no re-feed; forward seeks re-tile.
 */
class SubtitleFeeder {
  constructor(player, trackNumber, boundaries, sink) {
    this.player = player;
    this.trackNumber = trackNumber;
    this.boundaries = boundaries;
    this.sink = sink; // AssSubtitleController
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
    this.subs$ = null; // { feeder, trackNumber } for the active ASS track, or null
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

    await this.topUp(this.startBufferMs);
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

  // End (ms) of the buffered run that contains `currentMs`. If the playhead is in a
  // gap (e.g. just after a backward seek), returns `currentMs` so it reads as "below
  // target" and a feed is triggered — using the global last range here would stall.
  bufferedEndMs(stream, currentMs) {
    const r = stream.sb.buffered;
    for (let i = 0; i < r.length; i++) {
      const start = r.start(i) * 1000;
      const end = r.end(i) * 1000;
      if (currentMs >= start - 100 && currentMs < end) return end;
    }
    return currentMs;
  }

  async topUp(aheadMs = this.bufferAheadMs) {
    if (this.topUpQueued) return;
    this.topUpQueued = true;
    try {
      const currentMs = this.video.currentTime * 1000;
      for (const stream of [this.video$, this.audio$]) {
        if (!stream) continue;
        // Buffer transcoded audio further ahead in steady state, but not during the small
        // initial fill (aheadMs <= startBufferMs) so playback still starts promptly.
        const streamAhead =
          stream.transcoded && aheadMs > this.startBufferMs
            ? Math.max(aheadMs, this.transcodeBufferAheadMs)
            : aheadMs;
        const targetMs = currentMs + streamAhead;
        let guard = 0;
        while (
          !stream.feeder.done &&
          this.bufferedEndMs(stream, currentMs) < targetMs &&
          guard++ < 64
        ) {
          await stream.feeder.feedOne();
        }
      }
      // Subtitles ride the same sequential chain (preserving the source's single-in-flight
      // -read invariant). Cues are tiny; fetch windows a bit further ahead than media.
      if (this.subs$) {
        let guard = 0;
        const subTarget = currentMs + aheadMs + 4000;
        while (!this.subs$.feeder.done && this.subs$.feeder.fedThroughMs() < subTarget && guard++ < 128) {
          await this.subs$.feeder.feedOne();
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
    if (this.subs$) this.subs$.feeder.seekTo(t);
    this.topUp();
  }

  /** Begin streaming subtitle cues for `trackNumber` into `sink` (AssSubtitleController). */
  setSubtitleTrack(trackNumber, sink) {
    const feeder = new SubtitleFeeder(this.player, trackNumber, this.boundaries, sink);
    feeder.seekTo(this.video.currentTime * 1000);
    this.subs$ = { trackNumber, feeder };
    this.topUp();
  }

  clearSubtitleTrack() {
    this.subs$ = null;
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
