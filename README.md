# matroska.js

An experimental Matroska / EBML toolkit written in Rust, compiled to WebAssembly
so it can parse `.mkv` / `.webm` files **directly in the browser** — nothing is
uploaded.

> ⚠️ **Early prototype.** APIs are unstable, some EBML data types are still
> mis-decoded (notably floats), and there are rough edges throughout. Expect bugs.

## Goal

This is the **first step** toward a full MKV web player. Combined with
[libass](https://github.com/libass/libass) for subtitle rendering, the aim is to
play *most* `.mkv` video files directly in the browser — as long as the underlying
codecs are web-compatible — with **switchable video, audio, and subtitle tracks**.

Right now it parses and inspects the container; playback comes later.

## What's in here

This is a Cargo workspace + a small web frontend:

| Crate / dir   | What it is                                                                                     |
| ------------- | ---------------------------------------------------------------------------------------------- |
| `ebml-spec`   | A proc-macro that ingests the official EBML/Matroska schema XML **at compile time**, so every element ID knows its name and type. |
| `ebml-wasm`   | The EBML reader, exposed to JS via `wasm-bindgen`. A forward-only **async iterator**: call `.next().await` to get the next `EbmlElement` (id, offset, size, payload). Master elements yield a nested iterator, so you can skip whole subtrees without reading them. |
| `matroska-web`| A browser demo UI: drop a local video file and explore its EBML structure as a tree with a hex inspector, plus a quick metadata summary (tracks, languages, resolution, duration). |

## Build & run

You'll need the Rust toolchain and [`wasm-pack`](https://rustwasm.github.io/wasm-pack/).

```sh
# from ebml-wasm/: build the wasm module and copy it into the web frontend
cd ebml-wasm
wasm-pack build --target web && cp -r ./pkg ../matroska-web/

# then serve the frontend (any static server works)
cd ../matroska-web
npx simple-http-server -i --cors --port 8501
# open http://localhost:8501 and drop in an .mkv / .webm file
```

## License

**GNU Affero General Public License v3.0** (AGPL-3.0) — see [`LICENSE.txt`](LICENSE.txt).

You're free to use, study, modify, and share this. The catch: if you distribute
it **or run a modified version as a network service**, you must release your
source under the same license. In short — fork it all you want, but your changes
stay open too.
