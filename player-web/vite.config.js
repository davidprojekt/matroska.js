import { defineConfig } from 'vite';

export default defineConfig({
  // The wasm glue resolves its `.wasm` via `new URL('…', import.meta.url)`; keeping
  // ebml-wasm out of esbuild's dep pre-bundling preserves that reference so Vite
  // emits the wasm as an asset correctly.
  optimizeDeps: {
    // webtorrent is imported as its prebuilt browser bundle (dist/webtorrent.min.js),
    // which already inlines its Buffer/process polyfills — let it pass through untouched.
    // jassub resolves its module worker + wasm via `new Worker(new URL(…, import.meta.url))`;
    // excluding it from esbuild pre-bundling preserves those references so Vite bundles the
    // worker and emits the wasm itself (no explicit workerUrl/wasmUrl needed).
    exclude: ['ebml-wasm', 'webtorrent', 'jassub'],
    // jassub itself is excluded (above) to keep its `new URL(import.meta.url)` worker/wasm
    // references intact — but that also stops Vite pre-bundling its CommonJS deps, which
    // then get served raw and fail as `import x from 'cjs'` (no ESM default). Force-bundle
    // those specific CJS deps so they get proper interop. (abslink is already ESM.)
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
