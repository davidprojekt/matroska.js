// Run with: node --test src/mse.test.js
// Covers the gap-bridging buffer measurement that fixes the transcoded-audio
// infinite-buffering stall (see bufferedEndAcrossGaps in mse.js).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bufferedEndAcrossGaps, chooseFeedTarget, Pump, MseController } from './mse.js';

const video = (o) => ({ id: 'video', prioritized: true, deficit: 0, pumpFull: false, done: false, ...o });
const audio = (o) => ({ id: 'audio', prioritized: false, deficit: 0, pumpFull: false, done: false, ...o });

// Build a TimeRanges-like object from [startSec, endSec] pairs.
const ranges = (pairs) => ({
  length: pairs.length,
  start: (i) => pairs[i][0],
  end: (i) => pairs[i][1],
});

const TOL = 500;

test('single contiguous range: returns its end', () => {
  assert.equal(bufferedEndAcrossGaps(ranges([[0, 22.022]]), 22016, TOL), 22022);
});

test('playhead in a real gap returns currentMs (triggers a feed)', () => {
  // Playhead at 30s, buffer only covers [0,22] and [40,42].
  assert.equal(bufferedEndAcrossGaps(ranges([[0, 22], [40, 42]]), 30000, TOL), 30000);
});

test('bridges many small transcoded-AAC gaps into one run', () => {
  // The reported failure: 38 fragments, ~140ms holes between each. Strict
  // single-range logic would stop at the first hole (~1.846s); bridging must
  // reach the far end so the fill loop can hit its target and stop.
  const pairs = [];
  let t = 0;
  for (let i = 0; i < 38; i++) {
    pairs.push([+t.toFixed(3), +(t + 1.85).toFixed(3)]);
    t += 1.85 + 0.15; // 150ms hole, well under the 500ms tolerance
  }
  const farEnd = pairs[pairs.length - 1][1] * 1000;
  assert.equal(bufferedEndAcrossGaps(ranges(pairs), 22016, TOL), farEnd);
});

test('does NOT bridge a hole larger than the tolerance', () => {
  // A genuinely missing segment (2s hole) must end the run so we keep feeding.
  assert.equal(bufferedEndAcrossGaps(ranges([[0, 22], [24, 30]]), 10000, TOL), 22000);
});

test('bridges a hole exactly at the tolerance, not one past it', () => {
  assert.equal(bufferedEndAcrossGaps(ranges([[0, 10], [10.5, 12]]), 5000, TOL), 12000);
  assert.equal(bufferedEndAcrossGaps(ranges([[0, 10], [10.501, 12]]), 5000, TOL), 10000);
});

test('playhead just before a run (within 100ms) still counts as inside', () => {
  // Startup: currentTime 0 but first fragment begins at 0.05s.
  assert.equal(bufferedEndAcrossGaps(ranges([[0.05, 2.0]]), 0, TOL), 2000);
});

// --- chooseFeedTarget: the interleaving that stops audio from starving video ---

test('video is fed first whenever it has any deficit, even if tiny vs audio', () => {
  // The stall scenario: audio wants 18s more, video only 0.1s — video must win,
  // otherwise the slow audio transcode runs and video drains to a stall.
  const pick = chooseFeedTarget([video({ deficit: 100 }), audio({ deficit: 18000 })]);
  assert.equal(pick.id, 'video');
});

test('audio is fed only once video is at target', () => {
  const pick = chooseFeedTarget([video({ deficit: 0 }), audio({ deficit: 18000 })]);
  assert.equal(pick.id, 'audio');
});

test('a backpressured stream is skipped', () => {
  // Video pump is saturated (appends lagging) → feed audio instead of busy-spinning.
  const pick = chooseFeedTarget([video({ deficit: 5000, pumpFull: true }), audio({ deficit: 3000 })]);
  assert.equal(pick.id, 'audio');
});

test('a finished stream is skipped', () => {
  const pick = chooseFeedTarget([video({ deficit: 5000, done: true }), audio({ deficit: 3000 })]);
  assert.equal(pick.id, 'audio');
});

test('returns null when nothing needs feeding (loop terminates)', () => {
  assert.equal(chooseFeedTarget([video({ deficit: 0 }), audio({ deficit: -200 })]), null);
});

test('between two non-prioritized streams, larger deficit wins', () => {
  const pick = chooseFeedTarget([audio({ id: 'a1', deficit: 1000 }), audio({ id: 'a2', deficit: 4000 })]);
  assert.equal(pick.id, 'a2');
});

// --- SourceBuffer eviction / quota-aware retry (the "MediaSource buffer not
// sufficient" appendBuffer stall) --------------------------------------------

// Minimal SourceBuffer stand-in: appends can be primed to throw QuotaExceededError,
// and operations complete synchronously via _complete() (fires the updateend listener).
class FakeSB {
  constructor() {
    this.updating = false;
    this.timestampOffset = 0;
    this.ops = [];
    this._listeners = [];
    this.quotaAppends = 0; // number of upcoming appends that should throw quota
    this.buffered = { length: 0, start: () => 0, end: () => 0 };
  }
  addEventListener(type, fn) {
    if (type === 'updateend') this._listeners.push(fn);
  }
  appendBuffer(buf) {
    if (this.quotaAppends > 0) {
      this.quotaAppends -= 1;
      const e = new Error('SourceBuffer full');
      e.name = 'QuotaExceededError';
      throw e;
    }
    this.ops.push({ append: buf });
    this.updating = true;
  }
  remove(start, end) {
    this.ops.push({ remove: [start, end] });
    this.updating = true;
  }
  _complete() {
    this.updating = false;
    for (const fn of this._listeners.slice()) fn();
  }
}

test('quota on append evicts old data then retries the same segment', () => {
  const sb = new FakeSB();
  sb.quotaAppends = 1;
  const pump = new Pump(sb);
  pump.onQuotaExceeded = () => [0, 10];
  pump.push('seg');
  // First append threw quota → an eviction was spliced in front and executed.
  assert.deepEqual(sb.ops, [{ remove: [0, 10] }]);
  assert.equal(pump.queue.length, 1); // the segment is still queued for retry
  sb._complete(); // remove finishes → retry the append, which now succeeds
  assert.deepEqual(sb.ops, [{ remove: [0, 10] }, { append: 'seg' }]);
  assert.equal(pump.queue.length, 0);
});

test('quota with nothing to evict drops the segment (no wedged pump)', () => {
  const sb = new FakeSB();
  sb.quotaAppends = 1;
  const pump = new Pump(sb);
  pump.onQuotaExceeded = () => null;
  pump.push('seg');
  assert.deepEqual(sb.ops, []); // nothing appended, nothing evicted
  assert.equal(pump.queue.length, 0); // segment dropped rather than retried forever
});

test('quota that persists after eviction drops the segment after one retry', () => {
  const sb = new FakeSB();
  sb.quotaAppends = 2; // still full even after we free space
  const pump = new Pump(sb);
  pump.onQuotaExceeded = () => [0, 10];
  pump.push('seg');
  assert.deepEqual(sb.ops, [{ remove: [0, 10] }]);
  sb._complete(); // remove done → retry append → quota again → give up
  assert.deepEqual(sb.ops, [{ remove: [0, 10] }]); // no second eviction, no append
  assert.equal(pump.queue.length, 0);
});

test('a remove enqueued behind an append runs in order', () => {
  const sb = new FakeSB();
  const pump = new Pump(sb);
  pump.push('seg');
  pump.pushRemove(0, 5);
  assert.equal(pump.hasPendingRemove(), true);
  assert.deepEqual(sb.ops, [{ append: 'seg' }]); // append started; remove waits its turn
  sb._complete();
  assert.deepEqual(sb.ops, [{ append: 'seg' }, { remove: [0, 5] }]);
  assert.equal(pump.hasPendingRemove(), false);
});

// evictionRange is a pure controller method (reads this.video.currentTime + sb.buffered),
// so it can be exercised on a bare prototype instance without a real MediaSource.
const controllerAt = (currentTime) =>
  Object.assign(Object.create(MseController.prototype), { video: { currentTime } });
const buffered = (pairs) => ({
  length: pairs.length,
  start: (i) => pairs[i][0],
  end: (i) => pairs[i][1],
});

test('evictionRange trims everything up to keepBehind before the playhead', () => {
  const c = controllerAt(100);
  assert.deepEqual(c.evictionRange({ buffered: buffered([[0, 95]]) }, 30), [0, 70]);
});

test('evictionRange returns null near the start (nothing old enough)', () => {
  const c = controllerAt(20);
  assert.equal(c.evictionRange({ buffered: buffered([[0, 25]]) }, 30), null);
});

test('evictionRange returns null when the old data was already evicted', () => {
  const c = controllerAt(100);
  // Buffer already starts at 80, past the cutoff (70) → nothing left to reclaim.
  assert.equal(c.evictionRange({ buffered: buffered([[80, 95]]) }, 30), null);
});
