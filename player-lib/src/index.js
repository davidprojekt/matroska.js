// Public entry point for mkv-player-ui.
//
//   import { createPlayer } from 'mkv-player-ui';
//   import 'mkv-player-ui/style.css';
//
//   const player = createPlayer(document.querySelector('#player'), {
//     controls: 'full',                 // 'full' | 'minimal' | 'none' | { preset, ...perControlBooleans, dock }
//                                        // dock: 'overlay' (default, bar over the video) | 'below' (bar under it)
//     transcode: 'auto',                // 'auto' | true | false
//     title: 'My video',                // default title bar text; per-load override via load(url, { title })
//                                        // falls back to the MKV segment title, then the URL filename
//     watermark: 'Brand',               // bottom-right watermark: a string (shorthand for { text }),
//                                        // or { text, image, href }. image = logo URL; href = link.
//                                        // Always visible, fades/lifts with the controls (see style.css).
//     ffmpeg: { coreURL, wasmURL },     // where to load the ffmpeg.wasm core from (any CORS-enabled origin)
//     onStatus(msg, { level }) {},      // level: 'loading' | 'info'
//     onError(err) {},
//     onReady({ videoCodec, audioCodec, subtitleCount, durationMs }) {},
//     onTracks(tracks) {},
//   });
//   await player.load(url);
//   player.destroy();

import { MkvPlayer } from './player.js';

/**
 * Create a player, building its control bar into `container`. Returns a small frozen facade
 * (the full class is kept internal). Call `load(url)` to open a video and `destroy()` to
 * tear everything down.
 */
export function createPlayer(container, opts = {}) {
  const player = new MkvPlayer(container, opts);
  return Object.freeze({
    load: (url, loadOpts) => player.load(url, loadOpts),
    destroy: () => player.destroy(),
    on: (event, fn) => (player.on(event, fn), undefined),
    off: (event, fn) => (player.off(event, fn), undefined),
    /** The underlying <video> element, for embedders that need direct access. */
    get video() {
      return player.video;
    },
  });
}

export { resolveControls } from './controlBar.js';
