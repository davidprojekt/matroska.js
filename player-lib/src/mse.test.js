// Run with: node --test src/mse.test.js
// Covers the gap-bridging buffer measurement that fixes the transcoded-audio
// infinite-buffering stall (see bufferedEndAcrossGaps in mse.js).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bufferedEndAcrossGaps, chooseFeedTarget } from './mse.js';

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
