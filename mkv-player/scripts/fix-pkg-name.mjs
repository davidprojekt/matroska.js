// wasm-pack regenerates pkg/package.json on every build, deriving the package name from the
// crate name (`matroska-remux`). Rewrite it to the published npm identity `@matroska-js/remux`
// and attach publish metadata. Run as a build post-step — never hand-edit pkg/package.json, it
// is a build artifact that this script owns.
import { readFile, writeFile, copyFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const pkgPath = fileURLToPath(new URL('../pkg/package.json', import.meta.url));
const pkg = JSON.parse(await readFile(pkgPath, 'utf8'));

// wasm-pack does not copy the crate README into pkg/; do it here so the npm page has one.
await copyFile(
  fileURLToPath(new URL('../README.md', import.meta.url)),
  fileURLToPath(new URL('../pkg/README.md', import.meta.url)),
);
if (!pkg.files?.includes('README.md')) pkg.files = [...(pkg.files ?? []), 'README.md'];

pkg.name = '@matroska-js/remux';
pkg.description =
  'WebAssembly MKV/Matroska → fragmented-MP4 remuxer (the engine behind @matroska-js/player).';
pkg.repository = {
  type: 'git',
  url: 'git+https://github.com/davidprojekt/matroska.js.git',
  directory: 'mkv-player',
};
pkg.homepage = 'https://github.com/davidprojekt/matroska.js#readme';
// Scoped packages default to restricted; make publishes public without needing --access on the CLI.
pkg.publishConfig = { access: 'public' };

await writeFile(pkgPath, JSON.stringify(pkg, null, 2) + '\n');
console.log(`[fix-pkg-name] pkg/package.json name → ${pkg.name}`);
