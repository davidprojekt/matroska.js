import { createAppConfig } from '@nextcloud/vite-config';
import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

// The @matroska-js/player library ships as ESM *source* that this app's bundler must compile. It
// relies on three things the consumer build has to preserve: its wasm/worker
// `new URL(…, import.meta.url)` references, ES-module
// workers, and a couple of force-bundled CJS deps. We inject those via createAppConfig's
// `config` override so the Nextcloud build pipeline keeps them intact.

// Build with the ffmpeg transcoder code path present; no core is bundled — an admin supplies
// the core/wasm URLs at runtime (transcoding stays off until then).
const overrides = defineConfig({
  worker: {
    // jassub + ffmpeg spawn `{type:'module'}` workers; emit workers as ESM so prod matches dev.
    format: 'es',
  },
  optimizeDeps: {
    // Keep these out of esbuild pre-bundling so their `new URL(import.meta.url)` wasm/worker
    // references survive and Vite emits the assets itself.
    exclude: ['@matroska-js/remux', 'jassub', '@ffmpeg/ffmpeg', '@ffmpeg/util'],
    // jassub's CJS deps would otherwise be served raw and fail as ESM default-imports.
    include: ['throughput', 'rvfc-polyfill'],
  },
  define: {
    __TRANSCODE__: 'true',
  },
  server: {
    // Allow importing the workspace packages (@matroska-js/player, @matroska-js/remux pkg) that live one dir up.
    fs: { allow: ['..'] },
  },
  experimental: {
    // Override @nextcloud/vite-config's default, which resolves every built-asset URL through
    // `window.OC.filePath(...)`. That breaks the wasm/worker assets: the jassub and ffmpeg
    // module workers reference their own `.js`/`.wasm` via that same rewrite, but `window` does
    // not exist inside a Web Worker (the global is `self`), so the worker throws at startup.
    // Each entry/asset is served from the app dir and referenced relative to `import.meta.url`,
    // so relative resolution is correct in BOTH the main thread and workers — and needs no `OC`.
    renderBuiltUrl() {
      return { relative: true };
    },
  },
});

export default createAppConfig(
  {
    main: fileURLToPath(new URL('src/main.js', import.meta.url)),
    // Standalone Vue 3 app for the admin settings page (license key + buy link). Emitted as
    // js/matroskaplayer-admin-settings.mjs. It only loads on the admin settings page, isolated from
    // the Vue-2 Viewer handler in `main`, so bundling a Vue 3 runtime here causes no conflict.
    'admin-settings': fileURLToPath(new URL('src/admin-settings.js', import.meta.url)),
  },
  {
    config: overrides,
    // Disable @nextcloud/vite-config's default vite-plugin-node-polyfills. It rewrites the
    // @matroska-js/remux wasm-pack glue (which lives outside this app dir) to import a `global` shim
    // that then can't be resolved, breaking the build. The library builds with plain Vite
    // and no polyfills, so the glue doesn't need them at runtime.
    nodePolyfills: false,
  }
);
