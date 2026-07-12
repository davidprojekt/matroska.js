// Copies our custom-built ffmpeg.wasm core (see ../ffmpeg-core/) into the consuming app's
// public/ffmpeg/ so it is served same-origin — offline-first, no CDN. The core is LGPL, audio-only
// and royalty-free (built by `npm run build:ffmpeg` in @matroska-js/player); its LICENSE + SOURCE.md are
// copied alongside to satisfy the LGPL source-availability obligation.
//
// This is a SEPARATE, cached step from the app build: if the core hasn't been built yet, this
// warns and skips (the app still builds; transcoding is simply unavailable until the core exists).
// Invoked FROM each app's directory (its predev/prebuild hook), so it writes into that app's
// public/; the source is resolved relative to THIS script (inside the library).
import { existsSync, mkdirSync, copyFileSync, rmSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const cwd = process.cwd();
const scriptDir = dirname(fileURLToPath(import.meta.url));
const profile = process.env.FFMPEG_PROFILE || 'free-audio';
// Where the consuming app serves static files from. Vite apps use public/ffmpeg; the Nextcloud
// app serves from its own dir, so it passes FFMPEG_OUT_DIR=ffmpeg.
const outDir = resolve(cwd, process.env.FFMPEG_OUT_DIR || 'public/ffmpeg');
const files = ['ffmpeg-core.js', 'ffmpeg-core.wasm', 'LICENSE', 'SOURCE.md'];

// TRANSCODE=off builds ship no ffmpeg at all — clear any core copied by an earlier build so the
// 31 MB wasm can't leak into dist/.
if (process.env.TRANSCODE === 'off') {
  rmSync(outDir, { recursive: true, force: true });
  console.log('[ffmpeg] TRANSCODE=off — skipping core copy and clearing public/ffmpeg/.');
  process.exit(0);
}

const srcDir = resolve(scriptDir, '..', 'ffmpeg-core', 'dist', profile);
if (!existsSync(resolve(srcDir, 'ffmpeg-core.wasm'))) {
  console.warn(
    `[ffmpeg] no '${profile}' core at ${srcDir} — audio transcoding will be disabled.\n` +
      '[ffmpeg] build it with:  (cd path/to/@matroska-js/player && npm run build:ffmpeg)'
  );
  // Remove any stale core so the app doesn't serve a mismatched/old one.
  rmSync(outDir, { recursive: true, force: true });
  process.exit(0);
}

mkdirSync(outDir, { recursive: true });
for (const f of files) {
  const src = resolve(srcDir, f);
  if (existsSync(src)) copyFileSync(src, resolve(outDir, f));
}
console.log(`[ffmpeg] copied '${profile}' core (+ LICENSE, SOURCE.md) → ${outDir}`);
