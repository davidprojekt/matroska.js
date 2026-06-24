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
  push(buf) {
    this.queue.push(buf);
    this.flush();
  }
  flush() {
    if (this.sb.updating || this.queue.length === 0) return;
    try {
      this.sb.appendBuffer(this.queue.shift());
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

export class MseController {
  /**
   * @param player  MatroskaPlayer (wasm)
   * @param video   HTMLVideoElement
   * @param tracks  parsed track list (from player.tracks())
   * @param durationMs total duration
   * @param cueTimes  array of cue boundary times in ms (may be empty)
   */
  constructor(player, video, tracks, durationMs, cueTimes) {
    this.player = player;
    this.video = video;
    this.tracks = tracks;
    this.durationMs = durationMs;
    this.boundaries = buildBoundaries(cueTimes, durationMs);

    this.mediaSource = null;
    this.video$ = null; // { feeder, pump, track }
    this.audio$ = null;
    this.bufferAheadMs = 20000;
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

    await this.topUp();
  }

  setupTrack(track) {
    const mime = track.mime;
    if (!MediaSource.isTypeSupported(mime)) {
      console.warn(`unsupported: track ${track.number} ${mime}`);
      return null;
    }
    const sb = this.mediaSource.addSourceBuffer(mime);
    const pump = new Pump(sb);
    const init = this.player.init_segment(BIG(track.number));
    if (init && init.length) pump.push(init);
    const feeder = new TrackFeeder(this.player, track.number, this.boundaries, pump);
    return { track, sb, pump, feeder, mime };
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

  async topUp() {
    if (this.topUpQueued) return;
    this.topUpQueued = true;
    try {
      const currentMs = this.video.currentTime * 1000;
      const targetMs = currentMs + this.bufferAheadMs;
      for (const stream of [this.video$, this.audio$]) {
        if (!stream) continue;
        let guard = 0;
        while (
          !stream.feeder.done &&
          this.bufferedEndMs(stream, currentMs) < targetMs &&
          guard++ < 64
        ) {
          await stream.feeder.feedOne();
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
    this.topUp();
  }

  /** Switch the active audio track, rebuilding the audio buffer at the playhead. */
  async switchAudio(trackNumber) {
    const track = this.tracks.find((t) => t.number === trackNumber);
    if (!track || !this.audio$) return;
    const stream = this.audio$;

    // Drain anything queued and wait for the buffer to be idle.
    stream.pump.clear();
    await this.untilIdle(stream.sb);

    // Switch codec if needed, then clear the old audio and re-feed from the playhead.
    if (track.mime !== stream.mime && typeof stream.sb.changeType === 'function') {
      stream.sb.changeType(track.mime);
      stream.mime = track.mime;
    }
    if (stream.sb.buffered.length) {
      stream.sb.remove(0, Infinity);
      await this.untilIdle(stream.sb);
    }

    stream.track = track;
    stream.feeder = new TrackFeeder(this.player, trackNumber, this.boundaries, stream.pump);
    const init = this.player.init_segment(BIG(trackNumber));
    if (init && init.length) stream.pump.push(init);
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
