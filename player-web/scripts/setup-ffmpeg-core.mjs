// Copies the single-thread @ffmpeg/core ESM assets into public/ffmpeg/ so they are
// served same-origin. The ESM build is required because @ffmpeg/ffmpeg spawns a
// `{type:"module"}` worker that loads the core via dynamic `import()` and reads its
// `default` export — only the ESM build has `export default createFFmpegCore` (the UMD
// build exposes it via the CommonJS/global footer, which `import()` can't pick up).
// We copy rather than `import '@ffmpeg/core/...'` because the package's `exports` map
// doesn't expose the dist subpaths to the bundler. No-op when @ffmpeg/core isn't
// installed (transcoding-disabled builds), so the optional dependency stays optional.
import { existsSync, mkdirSync, copyFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const srcDir = resolve(root, 'node_modules/@ffmpeg/core/dist/esm');
const outDir = resolve(root, 'public/ffmpeg');
const files = ['ffmpeg-core.js', 'ffmpeg-core.wasm'];

if (!existsSync(resolve(srcDir, files[0]))) {
  console.log('[ffmpeg] @ffmpeg/core not installed — skipping core copy (transcoding disabled).');
  process.exit(0);
}
mkdirSync(outDir, { recursive: true });
for (const f of files) copyFileSync(resolve(srcDir, f), resolve(outDir, f));
console.log('[ffmpeg] copied single-thread core → public/ffmpeg/');
