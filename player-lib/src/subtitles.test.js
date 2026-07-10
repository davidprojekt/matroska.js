// Unit tests for the pure cue-cache logic of BitmapSubtitleTrack (PGS). The renderer
// classes need a DOM + libpgs/jassub and are exercised manually in the browser.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BitmapSubtitleTrack } from './subtitles.js';

const b64 = (arr) => Buffer.from(arr).toString('base64');

test('buildBuffer concatenates fragments in ascending-pts order', () => {
  const t = new BitmapSubtitleTrack();
  // Fed out of order (a forward seek can re-tile earlier windows later).
  t.addEvents([
    { pts: 2000, sup: b64([0x02, 0x03]) },
    { pts: 1000, sup: b64([0x01]) },
  ]);
  const bytes = [...new Uint8Array(t.buildBuffer())];
  assert.deepEqual(bytes, [0x01, 0x02, 0x03]);
});

test('dedups display sets by presentation timestamp', () => {
  const t = new BitmapSubtitleTrack();
  t.addEvents([{ pts: 1000, sup: b64([0x01]) }]);
  t.addEvents([{ pts: 1000, sup: b64([0x99]) }]); // same pts → dropped
  assert.equal(t.events.size, 1);
  assert.deepEqual([...new Uint8Array(t.buildBuffer())], [0x01]);
});

test('onChange fires once per batch that adds cues, not for no-op batches', () => {
  const t = new BitmapSubtitleTrack();
  let changes = 0;
  t.onChange = () => changes++;
  t.addEvents([{ pts: 1000, sup: b64([0x01]) }]); // +1
  t.addEvents([{ pts: 1000, sup: b64([0x02]) }]); // dup pts → no change
  t.addEvents([]); // empty → no change
  assert.equal(changes, 1);
});

test('empty cache builds a zero-length buffer', () => {
  const t = new BitmapSubtitleTrack();
  assert.equal(t.buildBuffer().byteLength, 0);
});
