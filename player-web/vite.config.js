import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

const transcodeOn = process.env.TRANSCODE !== 'off';
// The transcoding code (and the ffmpeg stub used to strip it) now lives in mkv-player-ui.
const stub = fileURLToPath(new URL('../player-lib/src/ffmpeg-stub.js', import.meta.url));

export default defineConfig({
  // Build-time switch for the ffmpeg.wasm audio-transcoding feature. `TRANSCODE=off`
  // folds this to `false`, so the dynamic import of ./audioTranscoder.js (the only
  // module that pulls in @ffmpeg/*) becomes dead code and is tree-shaken out.
  define: {
    __TRANSCODE__: JSON.stringify(transcodeOn),
  },
  // When disabled, alias @ffmpeg/* to an empty stub so a TRANSCODE=off build is completely
  // ffmpeg-free (otherwise Vite still emits the ffmpeg worker as a transform side effect).
  resolve: transcodeOn ? {} : { alias: { '@ffmpeg/ffmpeg': stub, '@ffmpeg/util': stub } },
  // The wasm glue resolves its `.wasm` via `new URL('…', import.meta.url)`; keeping
  // mkv-player out of esbuild's dep pre-bundling preserves that reference so Vite
  // emits the wasm as an asset correctly.
  optimizeDeps: {
    // jassub resolves its module worker + wasm via `new Worker(new URL(…, import.meta.url))`;
    // excluding it from esbuild pre-bundling preserves those references so Vite bundles the
    // worker and emits the wasm itself (no explicit workerUrl/wasmUrl needed).
    // @ffmpeg/ffmpeg likewise spawns a module worker via `new Worker(new URL(...))`.
    exclude: ['mkv-player', 'jassub', '@ffmpeg/ffmpeg', '@ffmpeg/util'],
    // jassub itself is excluded (above) to keep its `new URL(import.meta.url)` worker/wasm
    // references intact — but that also stops Vite pre-bundling its CommonJS deps, which
    // then get served raw and fail as `import x from 'cjs'` (no ESM default). Force-bundle
    // those specific CJS deps so they get proper interop. (abslink is already ESM.)
    // NB: jassub is a direct dependency of this app (not only of mkv-player-ui) so npm
    // hoists throughput/rvfc-polyfill here where esbuild can resolve them for this include.
    include: ['throughput', 'rvfc-polyfill'],
  },
  // jassub's worker is an ES module worker (`type: 'module'`); emit workers as ESM so the
  // production build matches (dev serves modules natively regardless).
  worker: {
    format: 'es',
  },
  server: {
    fs: {
      // Allow importing the wasm-pack output that lives outside this app dir.
      allow: ['..'],
    },
  },
});
