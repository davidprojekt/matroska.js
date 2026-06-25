// video.js v10 player UI (registers <video-player>, <video-skin> and all controls).
import '@videojs/html/video/skin';
import '@videojs/html/video/skin.css';

import initWasm, {MatroskaPlayer} from 'ebml-wasm';
import {MseController} from './mse.js';

const statusEl = document.getElementById('status');
const urlInput = document.getElementById('url');
const loadBtn = document.getElementById('load');
const audioSelect = document.getElementById('audio');
const subsSelect = document.getElementById('subs');
const video = document.querySelector('video-player video');

// Subtitle codecs we can turn into WebVTT today (ASS/SSA need libass — out of scope).
const TEXT_SUB_CODECS = new Set(['S_TEXT/UTF8', 'S_TEXT/WEBVTT', 'S_TEXT/ASCII']);

let activePlayer = null;
const loadedSubs = new Map(); // track number → HTMLTrackElement
const subtitleInfo = new Map(); // track number → { language, name }

const status = (msg) => {
  statusEl.textContent = msg;
  console.log('[player]', msg);
};

let wasmReady = false;
let controller = null;
let subtitleObjectUrls = [];

fileInput.addEventListener('change', (e) => {
  if (e.target.files && e.target.files[0]) {
    const file = e.target.files[0];
    url.value = URL.createObjectURL(file);
  }
});

const hash = window.location.hash.substring(1);

if (hash) {
    try {
        url.value = atob(hash);
    } catch (e) {
        console.error("The hash is not a valid Base64 encoded string:", e);
    }
}

async function preflight(url) {
  // The remuxer relies on HTTP byte ranges (206) and, cross-origin, on CORS. Probe
  // up front so a server that lacks them produces a clear message, not silent empty.
  let resp;
  try {
    resp = await fetch(url, { headers: { Range: 'bytes=0-1' } });
  } catch (e) {
    throw new Error(
      `Cannot fetch ${url} (${e.message}). If it's a different origin, the server needs CORS (Access-Control-Allow-Origin).`
    );
  }
  if (resp.status !== 206) {
    throw new Error(
      `Server returned ${resp.status} for a Range request (expected 206). ` +
        `Serve the file with byte-range support — e.g. the project's \`npm start\` (simple-http-server) on :8501.`
    );
  }
}

async function load(url) {
  status(`Opening ${url} …`);
  if (!wasmReady) {
    await initWasm();
    wasmReady = true;
  }

  await preflight(url);

  // Tear down any previous session.
  if (controller) {
    try {
      video.removeAttribute('src');
      video.load();
    } catch (_) {}
    controller = null;
  }
  subtitleObjectUrls.forEach((u) => URL.revokeObjectURL(u));
  subtitleObjectUrls = [];
  loadedSubs.clear();
  subtitleInfo.clear();
  for (const t of [...video.querySelectorAll('track')]) t.remove();
  audioSelect.innerHTML = '';
  subsSelect.innerHTML = '';

  const player = await MatroskaPlayer.open(url);
  activePlayer = player;
  const tracks = JSON.parse(player.tracks());
  const durationMs = Number(player.duration_ms());
  const cueTimes = JSON.parse(player.cue_times()).map(Number);

  const supported = (t) => t.mime && MediaSource.isTypeSupported(t.mime);
  const videoTracks = tracks.filter((t) => t.type === 'video');
  const audioTracks = tracks.filter((t) => t.type === 'audio');
  const subtitleTracks = tracks.filter((t) => t.type === 'subtitle');

  reportTracks(tracks);

  const videoTrack = videoTracks.find(supported) || null;
  const defaultAudio = audioTracks.find((t) => t.default && supported(t)) || audioTracks.find(supported) || null;

  // Audio track menu (v10 has no audio-track feature, so this is custom).
  for (const t of audioTracks) {
    const opt = document.createElement('option');
    opt.value = String(t.number);
    const tag = supported(t) ? '' : ' [unsupported]';
    opt.textContent = `${t.language || '??'} — ${t.name || t.codec_id}${tag}`;
    opt.disabled = !supported(t);
    if (t === defaultAudio) opt.selected = true;
    audioSelect.appendChild(opt);
  }

  // Subtitles are loaded lazily on selection (extraction scans the file), so only
  // populate the menu here. ASS/SSA are listed but disabled (libass is out of scope).
  const offOpt = document.createElement('option');
  offOpt.value = '';
  offOpt.textContent = 'Off';
  offOpt.selected = true;
  subsSelect.appendChild(offOpt);
  for (const t of subtitleTracks) {
    subtitleInfo.set(t.number, { language: t.language, name: t.name });
    const opt = document.createElement('option');
    opt.value = String(t.number);
    const loadable = TEXT_SUB_CODECS.has(t.codec_id);
    opt.disabled = !loadable;
    const tag = loadable ? '' : ' [ASS — not supported yet]';
    opt.textContent = `${t.language || '??'}${t.name ? ' — ' + t.name : ''}${tag}`;
    subsSelect.appendChild(opt);
  }

  controller = new MseController(player, video, tracks, durationMs, cueTimes);
  await controller.start(videoTrack, defaultAudio);

  status(
    `Loaded. video=${videoTrack ? videoTrack.codec_string : 'none'} ` +
      `audio=${defaultAudio ? defaultAudio.codec_string : 'none'} ` +
      `subs=${subtitleTracks.length} duration=${(durationMs / 1000).toFixed(1)}s`
  );
}

// Extract one subtitle track to WebVTT and attach it as a <track>. The WASM scan
// reads the whole file, so this runs only when the user picks the track.
async function loadSubtitle(player, number) {
  const vtt = await player.subtitles(BigInt(number));
  if (!vtt) return null;
  const info = subtitleInfo.get(number) || {};
  const blob = new Blob([vtt], { type: 'text/vtt' });
  const objectUrl = URL.createObjectURL(blob);
  subtitleObjectUrls.push(objectUrl);
  const track = document.createElement('track');
  track.kind = 'subtitles';
  track.label = `${info.language || '??'}${info.name ? ' — ' + info.name : ''}`;
  track.srclang = (info.language || 'und').slice(0, 3);
  track.src = objectUrl;
  video.appendChild(track);
  loadedSubs.set(number, track);
  return track;
}

subsSelect.addEventListener('change', async () => {
  for (const tt of video.textTracks) tt.mode = 'disabled';
  const value = subsSelect.value;
  if (!value || !activePlayer) return;
  const number = Number(value);
  let el = loadedSubs.get(number);
  if (!el) {
    status('Extracting subtitles (one-time scan)…');
    try {
      el = await loadSubtitle(activePlayer, number);
    } catch (e) {
      console.error(e);
      status('Subtitle extraction failed: ' + e.message);
      return;
    }
    if (!el) {
      status('No subtitle cues found for that track.');
      return;
    }
    status('Subtitles ready.');
  }
  if (el.track) el.track.mode = 'showing';
});

function reportTracks(tracks) {
  const lines = tracks.map(
    (t) => `  #${t.number} ${t.type} ${t.codec_id} ${t.mime ? `(${t.mime})` : '(not muxable)'} lang=${t.language}`
  );
  console.log('Tracks:\n' + lines.join('\n'));
}

audioSelect.addEventListener('change', () => {
  if (controller) controller.switchAudio(Number(audioSelect.value));
});

loadBtn.addEventListener('click', () => {
  load(urlInput.value.trim()).catch((e) => {
    console.error(e);
    status('Error: ' + e.message);
  });
});

// Auto-load the default URL on startup.
if(urlInput.value.trim().length !== 0) {
  load(urlInput.value.trim()).catch((e) => {
    console.error(e);
    status('Error: ' + e.message + ' (is the file server running?)');
  });
}
