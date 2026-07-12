// The full MKV player demo: a URL box, a local-file picker, and a copy-shareable-link
// button around the @matroska-js/player player. The playback engine (remux, MSE, controls,
// subtitles, transcoding) all lives in the library; this shell is just the page chrome.
import { createPlayer } from '@matroska-js/player';
import '@matroska-js/player/style.css';

const statusEl = document.getElementById('status');
const urlInput = document.getElementById('url');
const loadBtn = document.getElementById('load');
const fileInput = document.getElementById('fileInput');

const status = (msg) => {
  statusEl.textContent = msg;
};

const player = createPlayer(document.querySelector('.stage'), {
  controls: { preset: 'full', dock: 'below' },
  transcode: 'auto',
  // Load the ffmpeg.wasm core from the jsDelivr CDN rather than a same-origin copy. jsDelivr
  // sends permissive CORS, which the library's toBlobURL fetch requires for a cross-origin
  // core. (Because of this there's no local public/ffmpeg/ copy step for this app.)
  ffmpeg: {
    coreURL: 'https://cdn.jsdelivr.net/npm/@ffmpeg/core@0.12.10/dist/esm/ffmpeg-core.js',
    wasmURL: 'https://cdn.jsdelivr.net/npm/@ffmpeg/core@0.12.10/dist/esm/ffmpeg-core.wasm',
  },
  onStatus: (msg) => status(msg),
  onError: (e) => status('Error: ' + e.message),
});

// Local file → object URL so the WASM remuxer can Range-request it like any other source.
// Remember the file's name: a blob: object URL carries no filename for the library to fall
// back to, so we pass it explicitly as the title.
let pickedFileName = null;
fileInput.addEventListener('change', (e) => {
  const file = e.target.files && e.target.files[0];
  if (file) {
    urlInput.value = URL.createObjectURL(file);
    pickedFileName = file.name;
  }
});

const load = (url) =>
  // Local files load from a blob: URL with no derivable filename, so show the picked name.
  // For http URLs we pass nothing and let the library use the MKV segment title / filename.
  player
    .load(url, { title: url.startsWith('blob:') ? pickedFileName : undefined })
    .catch(() => {}); // onError already renders the message

loadBtn.addEventListener('click', () => {
  const url = urlInput.value.trim();
  if (url) load(url);
});

// Auto-load whatever is in the URL box on startup (default sample, or a decoded #hash).
if (urlInput.value.trim().length !== 0) load(urlInput.value.trim());
