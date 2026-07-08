// video.js v10 player UI (registers <video-player>, <media-container> and all controls).
import '@videojs/html/video/skin';
import '@videojs/html/video/skin.css';
// The skin inlines its own SVGs, but our ejected markup uses <media-icon name="…">,
// which needs the element defined and the default icon set registered.
import '@videojs/html/icons/element/default';

import initWasm, {MatroskaPlayer} from 'mkv-player';
import {MseController} from './mse.js';
import {AssSubtitleController} from './subtitles.js';
import {TrackMenu} from './menu.js';
import {addTorrent, streamUrlFor} from './torrent.js';

const statusEl = document.getElementById('status');
const urlInput = document.getElementById('url');
const loadBtn = document.getElementById('load');
const copyBtn = document.getElementById('copy');
const video = document.querySelector('video-player video');

// Audio + subtitle selection live in the control bar (see index.html), not in <select>s.
const audioMenu = new TrackMenu(
  document.getElementById('audioTrigger'),
  document.getElementById('audioMenu'),
  (v) => onAudioSelect(Number(v))
);
const subsMenu = new TrackMenu(
  document.getElementById('subsTrigger'),
  document.getElementById('subsMenu'),
  (v) => onSubSelect(v)
);
const chapterMenu = new TrackMenu(
  document.getElementById('chaptersTrigger'),
  document.getElementById('chaptersMenu'),
  (v) => onChapterSelect(v)
);
const prevChapterBtn = document.getElementById('prevChapter');
const nextChapterBtn = document.getElementById('nextChapter');
const chapterMarkersEl = document.getElementById('chapterMarkers');
const magnetInput = document.getElementById('magnet');
const torrentFileInput = document.getElementById('torrentFile');
const fetchTorrentBtn = document.getElementById('fetchTorrent');
const torrentFilesLabel = document.getElementById('torrentFilesLabel');
const torrentFilesSelect = document.getElementById('torrentFiles');

// Plain-text subtitle codecs we extract to WebVTT and attach as a native <track>.
const TEXT_SUB_CODECS = new Set(['S_TEXT/UTF8', 'S_TEXT/WEBVTT', 'S_TEXT/ASCII']);
// ASS/SSA codecs rendered via libass (JASSUB) over a canvas overlay.
const ASS_SUB_CODECS = new Set(['S_TEXT/ASS', 'S_TEXT/SSA']);

const subKind = (t) =>
  ASS_SUB_CODECS.has(t.codec_id) ? 'ass' : TEXT_SUB_CODECS.has(t.codec_id) ? 'text' : null;

let activePlayer = null;
let assSubs = null; // AssSubtitleController for the current file
let trackList = []; // parsed tracks of the current file (for forced-sub matching)
let chapterList = []; // parsed chapters of the current file
let userChoseSub = false; // true once the user explicitly picks a subtitle/Off
const loadedSubs = new Map(); // track number → HTMLTrackElement (text path)
const subtitleInfo = new Map(); // track number → { language, name }
const subKindByNumber = new Map(); // track number → 'ass' | 'text'

const status = (msg) => {
  statusEl.textContent = msg;
  console.log('[player]', msg);
};

let wasmReady = false;
let controller = null;
let transcoder = null; // ffmpeg.wasm AudioTranscoder for the current file, or null
let subtitleObjectUrls = [];

// MSE mime of the transcoder's output (AAC-in-MP4). Kept in sync with audioTranscoder.js.
const TRANSCODE_OUT_MIME = 'audio/mp4; codecs="mp4a.40.2"';

fileInput.addEventListener('change', (e) => {
  if (e.target.files && e.target.files[0]) {
    const file = e.target.files[0];
    urlInput.value = URL.createObjectURL(file);
  }
});

copyBtn.addEventListener('click', () => {
  let hash = btoa(urlInput.value);
  let hashedUrl = new URL(window.location.href);
  hashedUrl.hash = '#' + hash;
  navigator.clipboard.writeText(hashedUrl.href);
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

async function load(url, { skipPreflight = false } = {}) {
  status(`Opening ${url} …`);
  if (!wasmReady) {
    await initWasm();
    wasmReady = true;
  }

  // The WebTorrent service-worker stream URL already supports byte ranges, and probing
  // it can stall while pieces are still arriving — so skip the preflight for that path.
  if (!skipPreflight) await preflight(url);

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
  subKindByNumber.clear();
  userChoseSub = false;
  if (assSubs) {
    assSubs.destroy();
    assSubs = null;
  }
  if (transcoder) {
    transcoder.destroy();
    transcoder = null;
  }
  for (const t of [...video.querySelectorAll('track')]) t.remove();
  audioMenu.setItems([]);
  audioMenu.setAvailable(false);
  subsMenu.setItems([]);
  subsMenu.setAvailable(false);
  chapterMenu.setItems([]);
  chapterMenu.setAvailable(false);
  chapterList = [];
  if (chapterMarkersEl) chapterMarkersEl.textContent = '';

  const player = await MatroskaPlayer.open(url);
  activePlayer = player;
  assSubs = new AssSubtitleController(video);
  const tracks = JSON.parse(player.tracks());
  trackList = tracks;
  chapterList = JSON.parse(player.chapters());
  const durationMs = Number(player.duration_ms());
  const cueTimes = JSON.parse(player.cue_times()).map(Number);

  const nativelySupported = (t) => t.mime && MediaSource.isTypeSupported(t.mime);
  const videoTracks = tracks.filter((t) => t.type === 'video');
  const audioTracks = tracks.filter((t) => t.type === 'audio');
  const subtitleTracks = tracks.filter((t) => t.type === 'subtitle');

  reportTracks(tracks);

  // Audio whose codec the browser can't decode natively can be transcoded in-browser with
  // ffmpeg.wasm (gated on the __TRANSCODE__ build flag and Opus-in-MP4 being playable).
  const canTranscode = __TRANSCODE__ && MediaSource.isTypeSupported(TRANSCODE_OUT_MIME);
  const audioPlayable = (t) => nativelySupported(t) || canTranscode;

  // Spin up the transcoder (lazily — it only downloads the ffmpeg core on first use) when
  // some audio track needs it. Confined to a dynamic import so a `TRANSCODE=off` build
  // tree-shakes @ffmpeg/* away entirely.
  if (canTranscode && audioTracks.some((t) => !nativelySupported(t))) {
    const { AudioTranscoder } = await import('./audioTranscoder.js');
    transcoder = new AudioTranscoder();
  }

  const videoTrack = videoTracks.find(nativelySupported) || null;
  // Prefer a natively-playable audio track (default first) so we only transcode when there's
  // no native option; otherwise fall back to the default/first track via transcoding.
  const defaultAudio =
    audioTracks.find((t) => t.default && nativelySupported(t)) ||
    audioTracks.find(nativelySupported) ||
    (canTranscode ? audioTracks.find((t) => t.default) || audioTracks[0] || null : null) ||
    null;

  // Audio track menu (v10 has no audio-track feature, so this is custom).
  audioMenu.setItems(
    audioTracks.map((t) => {
      const native = nativelySupported(t);
      const tag = native ? '' : canTranscode ? ' [transcoded]' : ' [unsupported]';
      return {
        value: String(t.number),
        label: `${t.language || '??'} — ${t.name || t.codec_id}${tag}`,
        disabled: !audioPlayable(t),
        selected: t === defaultAudio,
      };
    })
  );
  audioMenu.setAvailable(audioTracks.length > 0);

  // Chapter menu — titles in the starting audio's language (rebuilt on audio change).
  buildChapterMenu(defaultAudio ? defaultAudio.language : null);
  buildChapterMarkers(durationMs);

  // ASS tracks render via libass. Plain-text subs are listed but disabled (the WebVTT
  // path is not wired into the libass overlay yet).
  const subItems = [{ value: '', label: 'Off', selected: true }];
  for (const t of subtitleTracks) {
    subtitleInfo.set(t.number, { language: t.language, name: t.name });
    const kind = subKind(t);
    if (kind) subKindByNumber.set(t.number, kind);
    const tag = kind === 'ass' ? (t.forced ? ' [forced]' : '') : ` [${t.codec_id} — unsupported]`;
    subItems.push({
      value: String(t.number),
      label: `${t.language || '??'}${t.name ? ' — ' + t.name : ''}${tag}`,
      disabled: kind !== 'ass', // only ASS is wired up for now
    });
  }
  subsMenu.setItems(subItems);
  subsMenu.setAvailable(subtitleTracks.length > 0);

  controller = new MseController(player, video, tracks, durationMs, cueTimes, transcoder);
  if (defaultAudio && !nativelySupported(defaultAudio) && canTranscode) {
    status('Preparing audio transcoder… (first load downloads the decoder)');
  }
  await controller.start(videoTrack, defaultAudio);

  // Fonts download out-of-band (separate connections) so they don't disturb the single
  // forward media stream; subtitles render once they arrive. Fire and forget.
  loadFonts(player, url).catch((e) => console.warn('font loading failed', e));

  // Soft-force any forced subtitle matching the starting audio language.
  if (defaultAudio) applyForcedSubtitle(defaultAudio.language);

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

// Turn a subtitle selection on: '' = off, an ASS track = libass, a text track = WebVTT.
async function selectSubtitle(value) {
  // Reset both renderers first so only the chosen track is active.
  for (const tt of video.textTracks) tt.mode = 'disabled';
  if (assSubs) assSubs.disable();
  if (controller) controller.clearSubtitleTrack();
  if (!value || !activePlayer) return;

  const number = Number(value);
  const kind = subKindByNumber.get(number);
  if (kind === 'ass') {
    try {
      const header = activePlayer.subtitle_header(BigInt(number));
      await assSubs.enableTrack(header);
      controller.setSubtitleTrack(number, assSubs);
      status('ASS subtitles on (streaming).');
    } catch (e) {
      console.error(e);
      status('ASS subtitle error: ' + e.message);
    }
  } else if (kind === 'text') {
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
  }
}

// Menu callbacks (wired in the TrackMenu instances near the top).
function onAudioSelect(number) {
  if (controller) controller.switchAudio(number);
  const t = trackList.find((x) => x.number === number);
  if (t) {
    applyForcedSubtitle(t.language);
    buildChapterMenu(t.language); // re-pick chapter titles for the new audio language
  }
}

function onSubSelect(value) {
  userChoseSub = true; // explicit choice — forced-subtitle logic must not override it
  selectSubtitle(value);
}

// ---- Chapters ----

// Pick a chapter's title in the audio language (BCP-47 then ISO-639), else the first.
function pickChapterTitle(chapter, audioLang) {
  const displays = chapter.displays || [];
  if (audioLang) {
    const m = displays.find((d) => langMatch(d.languageBcp47 || d.language, audioLang));
    if (m) return m.text;
  }
  return (displays[0] && displays[0].text) || 'Chapter';
}

// ms → `m:ss` or `h:mm:ss`.
function fmtChapterTime(ms) {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

function buildChapterMenu(audioLang) {
  chapterMenu.setItems(
    chapterList.map((c) => ({
      value: String(c.startMs),
      label: `${fmtChapterTime(c.startMs)}  ${pickChapterTitle(c, audioLang)}`,
    }))
  );
  const has = chapterList.length > 0;
  chapterMenu.setAvailable(has);
  if (prevChapterBtn) prevChapterBtn.hidden = !has;
  if (nextChapterBtn) nextChapterBtn.hidden = !has;
  highlightCurrentChapter();
}

// Place a tick on the time slider at each chapter boundary (positioned by % of duration).
function buildChapterMarkers(durationMs) {
  if (!chapterMarkersEl) return;
  chapterMarkersEl.textContent = '';
  if (!chapterList.length || !(durationMs > 0)) return;
  for (const c of chapterList) {
    if (c.startMs <= 0 || c.startMs >= durationMs) continue; // skip the implicit start
    const tick = document.createElement('div');
    tick.className = 'vjs-chapter-marker';
    tick.style.left = `${(c.startMs / durationMs) * 100}%`;
    chapterMarkersEl.appendChild(tick);
  }
}

const seekTo = (seconds) => {
  if (Number.isFinite(seconds)) video.currentTime = Math.max(0, seconds);
};

function onChapterSelect(value) {
  seekTo(Number(value) / 1000);
}

// Index of the chapter containing `ms` (largest startMs ≤ ms), or -1 before the first.
function currentChapterIndex(ms) {
  let idx = -1;
  for (let i = 0; i < chapterList.length; i++) {
    if (chapterList[i].startMs <= ms + 1) idx = i;
    else break;
  }
  return idx;
}

// Highlight the current chapter in the menu as playback progresses.
function highlightCurrentChapter() {
  if (!chapterList.length) return;
  const idx = currentChapterIndex(video.currentTime * 1000);
  chapterMenu.setValue(idx >= 0 ? String(chapterList[idx].startMs) : null);
}

function goToNextChapter() {
  const ms = video.currentTime * 1000;
  const next = chapterList.find((c) => c.startMs > ms + 250);
  if (next) seekTo(next.startMs / 1000);
}

function goToPrevChapter() {
  const ms = video.currentTime * 1000;
  const idx = currentChapterIndex(ms);
  if (idx <= 0) {
    seekTo(0);
    return;
  }
  // >3s into the current chapter restarts it; otherwise jump to the previous one.
  const restart = ms - chapterList[idx].startMs > 3000;
  seekTo(chapterList[restart ? idx : idx - 1].startMs / 1000);
}

// Fetch font attachments out-of-band (one Range request each, parallel, on separate
// connections) so they never contend with the single forward media stream, and hand the
// bytes to libass. baseUrl is the same URL the demuxer plays from (HTTP, the torrent
// service-worker URL, or a blob: URL — all support Range).
async function loadFonts(player, baseUrl) {
  const sink = assSubs;
  let list;
  try {
    list = JSON.parse(player.font_attachments());
  } catch {
    return;
  }
  if (!list.length) return;
  status(`Loading ${list.length} font attachment(s)…`);
  await Promise.all(
    list.map(async (f) => {
      try {
        const resp = await fetch(baseUrl, { headers: { Range: `bytes=${f.start}-${f.end}` } });
        const buf = new Uint8Array(await resp.arrayBuffer());
        // If the server ignored Range and returned the whole body (200), slice ourselves.
        const data = resp.status === 206 ? buf : buf.slice(Number(f.start), Number(f.end) + 1);
        if (sink === assSubs) sink.addFontData(data); // ignore if a new file loaded meanwhile
      } catch (e) {
        console.warn(`font "${f.name}" fetch failed`, e);
      }
    })
  );
  status(`Fonts ready (${list.length}).`);
}

// Two language tags match if equal or share the same primary subtag (e.g. "jpn"/"ja").
function langMatch(a, b) {
  if (!a || !b) return false;
  a = a.toLowerCase();
  b = b.toLowerCase();
  return a === b || a.slice(0, 2) === b.slice(0, 2);
}

// Soft-force a forced subtitle for `audioLang` (foreign signs/songs) — but only if the
// user hasn't made their own subtitle choice.
function applyForcedSubtitle(audioLang) {
  if (userChoseSub) return;
  const forced = trackList.find(
    (t) =>
      t.type === 'subtitle' &&
      t.forced &&
      subKindByNumber.has(t.number) &&
      langMatch(t.language, audioLang)
  );
  if (!forced) return;
  subsMenu.setValue(String(forced.number));
  selectSubtitle(String(forced.number)); // programmatic — keep userChoseSub false
}

function reportTracks(tracks) {
  const lines = tracks.map(
    (t) => `  #${t.number} ${t.type} ${t.codec_id} ${t.mime ? `(${t.mime})` : '(not muxable)'} lang=${t.language}`
  );
  console.log('Tracks:\n' + lines.join('\n'));
}

prevChapterBtn.addEventListener('click', goToPrevChapter);
nextChapterBtn.addEventListener('click', goToNextChapter);
video.addEventListener('timeupdate', highlightCurrentChapter);

loadBtn.addEventListener('click', () => {
  load(urlInput.value.trim()).catch((e) => {
    console.error(e);
    status('Error: ' + e.message);
  });
});

// --- WebTorrent: fetch metadata, then let the user pick one file to play ---

const fmtSize = (bytes) => {
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0, n = bytes;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
};

let torrentFiles = [];

async function fetchTorrent() {
  const file = torrentFileInput.files && torrentFileInput.files[0];
  const source = file || magnetInput.value.trim();
  if (!source) {
    status('Paste a magnet link or choose a .torrent file first.');
    return;
  }

  fetchTorrentBtn.disabled = true;
  torrentFilesLabel.hidden = true;
  torrentFilesSelect.innerHTML = '';
  torrentFiles = [];

  // Browser peers may be slow/absent — nudge the user instead of looking frozen.
  const slowWarn = setTimeout(() => {
    status('Still fetching metadata… no WebRTC peers yet. This torrent may not be web-seeded.');
  }, 20000);

  try {
    const torrent = await addTorrent(source, status);
    clearTimeout(slowWarn);
    torrentFiles = torrent.files.slice();
    if (!torrentFiles.length) {
      status('Torrent has no files.');
      return;
    }

    // Default the selection to the largest file (usually the video).
    let largest = 0;
    torrentFiles.forEach((f, i) => { if (f.length > torrentFiles[largest].length) largest = i; });

    const hint = document.createElement('option');
    hint.value = '';
    hint.textContent = '— select a file —';
    torrentFilesSelect.appendChild(hint);
    torrentFiles.forEach((f, i) => {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = `${f.name} (${fmtSize(f.length)})`;
      if (i === largest) opt.selected = true;
      torrentFilesSelect.appendChild(opt);
    });
    torrentFilesLabel.hidden = false;
    status(`Metadata ready: ${torrentFiles.length} file(s). Pick one to play.`);
  } catch (e) {
    clearTimeout(slowWarn);
    console.error(e);
    status('Torrent error: ' + e.message);
  } finally {
    fetchTorrentBtn.disabled = false;
  }
}

fetchTorrentBtn.addEventListener('click', fetchTorrent);

torrentFilesSelect.addEventListener('change', () => {
  const value = torrentFilesSelect.value;
  if (value === '') return;
  const file = torrentFiles[Number(value)];
  if (!file) return;
  // streamURL is a root-relative path (/webtorrent/…); make it absolute so the WASM
  // HTTP client gets the same shape it always has (it only sees absolute URLs). The
  // service worker (scope '/') still serves it.
  const streamUrl = new URL(streamUrlFor(file), location.origin).href;
  urlInput.value = streamUrl; // reflect it in the URL box for visibility
  status(`Streaming "${file.name}" from torrent…`);
  load(streamUrl, { skipPreflight: true }).catch((e) => {
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
