// Copies the single-thread @ffmpeg/core ESM assets into <cwd>/public/ffmpeg/ so they are
// served same-origin by the consuming app. The ESM build is required because @ffmpeg/ffmpeg
// spawns a `{type:"module"}` worker that loads the core via dynamic `import()` and reads its
// `default` export — only the ESM build has `export default createFFmpegCore` (the UMD build
// exposes it via the CommonJS/global footer, which `import()` can't pick up). We copy rather
// than `import '@ffmpeg/core/...'` because the package's `exports` map doesn't expose the
// dist subpaths to the bundler.
//
// This script lives in the mkv-player-ui library but is invoked FROM each app's directory
// (its predev/prebuild hook: `node ../player-lib/scripts/setup-ffmpeg-core.mjs`), so it
// resolves paths against `process.cwd()` — the app — not the script's own location. It reads
// @ffmpeg/core from wherever the install placed it (hoisted to the app, or nested under the
// library's node_modules) and writes into the app's public/. No-op when @ffmpeg/core isn't
// installed (transcoding-disabled builds), so the optional dependency stays optional.
import { existsSync, mkdirSync, copyFileSync, rmSync } from 'node:fs';
import { resolve } from 'node:path';

const cwd = process.cwd();
const files = ['ffmpeg-core.js', 'ffmpeg-core.wasm'];
const outDir = resolve(cwd, 'public/ffmpeg');

// TRANSCODE=off builds ship no ffmpeg at all — don't copy the core into public/ (Vite would
// then emit the 31 MB wasm into dist/ even though the JS bundle is transcode-free). Also
// remove any core left in public/ from an earlier transcode-enabled build so it can't leak.
if (process.env.TRANSCODE === 'off') {
  rmSync(outDir, { recursive: true, force: true });
  console.log('[ffmpeg] TRANSCODE=off — skipping core copy and clearing public/ffmpeg/.');
  process.exit(0);
}

// Candidate locations for the installed core, in resolution-priority order: the app's own
// node_modules (npm usually hoists transitive deps here), then nested under the library.
const candidates = [
  resolve(cwd, 'node_modules/@ffmpeg/core/dist/esm'),
  resolve(cwd, 'node_modules/mkv-player-ui/node_modules/@ffmpeg/core/dist/esm'),
];
const srcDir = candidates.find((d) => existsSync(resolve(d, files[0])));

if (!srcDir) {
  console.log('[ffmpeg] @ffmpeg/core not installed — skipping core copy (transcoding disabled).');
  process.exit(0);
}

mkdirSync(outDir, { recursive: true });
for (const f of files) copyFileSync(resolve(srcDir, f), resolve(outDir, f));
console.log('[ffmpeg] copied single-thread core → public/ffmpeg/');
