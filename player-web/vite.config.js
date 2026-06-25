import { defineConfig } from 'vite';

export default defineConfig({
  // The wasm glue resolves its `.wasm` via `new URL('…', import.meta.url)`; keeping
  // ebml-wasm out of esbuild's dep pre-bundling preserves that reference so Vite
  // emits the wasm as an asset correctly.
  optimizeDeps: {
    // webtorrent is imported as its prebuilt browser bundle (dist/webtorrent.min.js),
    // which already inlines its Buffer/process polyfills — let it pass through untouched.
    exclude: ['ebml-wasm', 'webtorrent'],
  },
  server: {
    fs: {
      // Allow importing the wasm-pack output that lives outside this app dir.
      allow: ['..'],
    },
  },
});
