// The player orchestrator: opens an MKV via the WASM remuxer, drives MSE playback, and
// wires the control-bar track/chapter/subtitle menus. Extracted from the demo apps' main.js
// so both the embed and the full web app share one implementation. All DOM access goes
// through the `refs` returned by buildControlBar (scoped, no global ids), and all user-facing
// messages are emitted as events so each app renders its own status chrome.

import '@videojs/html/video/skin';
import '@videojs/html/video/skin.css';
// The skin inlines its own SVGs, but our ejected markup uses <media-icon name="…">, which
// needs the element defined and the default icon set registered.
import '@videojs/html/icons/element/default';

import initWasm, { MatroskaPlayer } from 'mkv-player';
import { MseController } from './mse.js';
import { AssSubtitleController } from './subtitles.js';
import { TrackMenu } from './menu.js';
import { buildControlBar } from './controlBar.js';

// Plain-text subtitle codecs we extract to WebVTT and attach as a native <track>.
const TEXT_SUB_CODECS = new Set(['S_TEXT/UTF8', 'S_TEXT/WEBVTT', 'S_TEXT/ASCII']);
// ASS/SSA codecs rendered via libass (JASSUB) over a canvas overlay.
const ASS_SUB_CODECS = new Set(['S_TEXT/ASS', 'S_TEXT/SSA']);

// MSE mime of the transcoder's output (AAC-in-MP4). Kept in sync with audioTranscoder.js.
// Hardcoded (rather than imported) so referencing it doesn't pull @ffmpeg/* into the main
// chunk — the transcoder module is only reached via a dynamic import.
const TRANSCODE_OUT_MIME = 'audio/mp4; codecs="mp4a.40.2"';

const subKind = (t) =>
  ASS_SUB_CODECS.has(t.codec_id) ? 'ass' : TEXT_SUB_CODECS.has(t.codec_id) ? 'text' : null;

// Init the WASM module once per page, no matter how many players exist.
let wasmReady = null;
const ensureWasm = () => (wasmReady ||= initWasm());

// Two language tags match if equal or share the same primary subtag (e.g. "jpn"/"ja").
function langMatch(a, b) {
  if (!a || !b) return false;
  a = a.toLowerCase();
  b = b.toLowerCase();
  return a === b || a.slice(0, 2) === b.slice(0, 2);
}

export class MkvPlayer {
  constructor(container, opts = {}) {
    if (!container) throw new Error('createPlayer requires a container element');
    this.container = container;
    this.destroyed = false;

    // --- event emitter (callbacks in opts are sugar over on/off) ---
    this._listeners = { status: [], error: [], ready: [], tracks: [] };
    if (opts.onStatus) this.on('status', opts.onStatus);
    if (opts.onError) this.on('error', opts.onError);
    if (opts.onReady) this.on('ready', opts.onReady);
    if (opts.onTracks) this.on('tracks', opts.onTracks);

    // --- ffmpeg / transcoding config ---
    const base = opts.baseURL ?? import.meta?.env?.BASE_URL ?? '/';
    this._coreURL = opts.ffmpeg?.coreURL ?? `${base}ffmpeg/ffmpeg-core.js`;
    this._wasmURL = opts.ffmpeg?.wasmURL ?? `${base}ffmpeg/ffmpeg-core.wasm`;
    // `transcode`: 'auto' (default) honors the __TRANSCODE__ build define if the consuming
    // app sets one, else defaults on; `true`/`false` force it. The typeof guard means a
    // published-library consumer that never defines __TRANSCODE__ still works.
    const buildFlag = typeof __TRANSCODE__ !== 'undefined' ? __TRANSCODE__ : true;
    const t = opts.transcode ?? 'auto';
    this._wantTranscode = t === 'auto' ? buildFlag : !!t;

    // Default title shown in the title bar for every load, unless a per-load `title` overrides
    // it. When neither is given the library falls back to the MKV segment title, then the URL
    // filename (see _load).
    this._title = opts.title ?? null;

    // --- build the control bar and capture element references ---
    const refs = buildControlBar(container, opts.controls);
    this.refs = refs;
    this.video = refs.video;

    // Track menus live in the control bar; each exists only if that control is enabled.
    this.audioMenu = refs.audioTrigger
      ? new TrackMenu(refs.audioTrigger, refs.audioMenu, (v) => this._onAudioSelect(Number(v)))
      : null;
    this.subsMenu = refs.subsTrigger
      ? new TrackMenu(refs.subsTrigger, refs.subsMenu, (v) => this._onSubSelect(v))
      : null;
    this.chapterMenu = refs.chaptersTrigger
      ? new TrackMenu(refs.chaptersTrigger, refs.chaptersMenu, (v) => this._onChapterSelect(v))
      : null;

    // --- per-session state ---
    this.activePlayer = null;
    this.assSubs = null;
    this.trackList = [];
    this.chapterList = [];
    this.userChoseSub = false;
    this.loadedSubs = new Map(); // track number → HTMLTrackElement (text path)
    this.subtitleInfo = new Map(); // track number → { language, name }
    this.subKindByNumber = new Map(); // track number → 'ass' | 'text'
    this.controller = null;
    this.transcoder = null;
    this.subtitleObjectUrls = [];

    // Bound handlers (stored so destroy() can detach them).
    this._onPrevChapter = () => this._goToPrevChapter();
    this._onNextChapter = () => this._goToNextChapter();
    this._onTimeUpdate = () => this._highlightCurrentChapter();
    if (refs.prevChapter) refs.prevChapter.addEventListener('click', this._onPrevChapter);
    if (refs.nextChapter) refs.nextChapter.addEventListener('click', this._onNextChapter);
    this.video.addEventListener('timeupdate', this._onTimeUpdate);
  }

  // ---- events ----
  on(event, fn) {
    (this._listeners[event] ||= []).push(fn);
    return this;
  }
  off(event, fn) {
    const list = this._listeners[event];
    if (list) this._listeners[event] = list.filter((f) => f !== fn);
    return this;
  }
  _emit(event, ...args) {
    for (const fn of this._listeners[event] || []) {
      try {
        fn(...args);
      } catch (e) {
        console.error(e);
      }
    }
  }
  // Emit a status message. level: 'loading' (blocking work, before playback starts) or
  // 'info' (incidental — apps may choose to ignore these once playing).
  _status(msg, level = 'info') {
    console.log('[player]', msg);
    this._emit('status', msg, { level });
  }

  // Filename component of a URL, for use as a fallback title. Returns null for blob:/data:
  // URLs (object URLs carry no meaningful name) or anything unparseable.
  _basenameFromUrl(url) {
    try {
      const u = new URL(url, window.location.href);
      if (u.protocol === 'blob:' || u.protocol === 'data:') return null;
      const base = decodeURIComponent((u.pathname.split('/').pop() || '').trim());
      return base || null;
    } catch {
      return null;
    }
  }

  // Write the title bar text and hide the bar entirely when there's nothing to show.
  _setTitle(title) {
    const el = this.refs.titleBar;
    if (!el) return;
    const text = title || '';
    el.textContent = text;
    el.hidden = !text;
  }

  async _preflight(url) {
    // The remuxer relies on HTTP byte ranges (206) and, cross-origin, on CORS. Probe up
    // front so a server that lacks them produces a clear message, not silent empty.
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
          `The video must be served with byte-range support.`
      );
    }
  }

  async load(url, { skipPreflight = false, title } = {}) {
    try {
      await this._load(url, { skipPreflight, title });
    } catch (e) {
      console.error(e);
      this._emit('error', e);
      throw e;
    }
  }

  async _load(url, { skipPreflight, title }) {
    if (this.destroyed) throw new Error('player destroyed');
    this._status(`Opening ${url} …`, 'loading');
    await ensureWasm();

    if (!skipPreflight) await this._preflight(url);

    this._teardownSession();

    const player = await MatroskaPlayer.open(url);
    this.activePlayer = player;

    // Title precedence: explicit per-load title → constructor default → MKV segment title →
    // URL filename. `player.title()` returns undefined when the file carries no Info\Title.
    this._setTitle(title ?? this._title ?? player.title() ?? this._basenameFromUrl(url));
    this.assSubs = new AssSubtitleController(this.video);
    const tracks = JSON.parse(player.tracks());
    this.trackList = tracks;
    this.chapterList = JSON.parse(player.chapters());
    const durationMs = Number(player.duration_ms());
    const cueTimes = JSON.parse(player.cue_times()).map(Number);

    const nativelySupported = (t) => t.mime && MediaSource.isTypeSupported(t.mime);
    const videoTracks = tracks.filter((t) => t.type === 'video');
    const audioTracks = tracks.filter((t) => t.type === 'audio');
    const subtitleTracks = tracks.filter((t) => t.type === 'subtitle');

    this._reportTracks(tracks);
    this._emit('tracks', tracks);

    // Audio whose codec the browser can't decode natively can be transcoded in-browser with
    // ffmpeg.wasm (gated on the `transcode` option and Opus/AAC-in-MP4 being playable).
    const canTranscode = this._wantTranscode && MediaSource.isTypeSupported(TRANSCODE_OUT_MIME);
    const audioPlayable = (t) => nativelySupported(t) || canTranscode;

    // Spin up the transcoder (lazily — it only downloads the ffmpeg core on first use) when
    // some audio track needs it. Confined to a dynamic import so a `transcode:false` build
    // never pulls @ffmpeg/* at runtime (and a TRANSCODE=off app build tree-shakes it away).
    if (canTranscode && audioTracks.some((t) => !nativelySupported(t))) {
      const { AudioTranscoder } = await import('./audioTranscoder.js');
      this.transcoder = new AudioTranscoder({ coreURL: this._coreURL, wasmURL: this._wasmURL });
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
    if (this.audioMenu) {
      this.audioMenu.setItems(
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
      this.audioMenu.setAvailable(audioTracks.length > 0);
    }

    // Chapter menu — titles in the starting audio's language (rebuilt on audio change).
    this._buildChapterMenu(defaultAudio ? defaultAudio.language : null);
    this._buildChapterMarkers(durationMs);

    // ASS tracks render via libass. Plain-text subs are listed but disabled (the WebVTT path
    // is not wired into the libass overlay yet).
    if (this.subsMenu) {
      const subItems = [{ value: '', label: 'Off', selected: true }];
      for (const t of subtitleTracks) {
        this.subtitleInfo.set(t.number, { language: t.language, name: t.name });
        const kind = subKind(t);
        if (kind) this.subKindByNumber.set(t.number, kind);
        const tag =
          kind === 'ass' ? (t.forced ? ' [forced]' : '') : ` [${t.codec_id} — unsupported]`;
        subItems.push({
          value: String(t.number),
          label: `${t.language || '??'}${t.name ? ' — ' + t.name : ''}${tag}`,
          disabled: kind !== 'ass', // only ASS is wired up for now
        });
      }
      this.subsMenu.setItems(subItems);
      this.subsMenu.setAvailable(subtitleTracks.length > 0);
    } else {
      // Menu hidden, but forced-subtitle logic still needs the codec/kind maps.
      for (const t of subtitleTracks) {
        this.subtitleInfo.set(t.number, { language: t.language, name: t.name });
        const kind = subKind(t);
        if (kind) this.subKindByNumber.set(t.number, kind);
      }
    }

    this.controller = new MseController(
      player,
      this.video,
      tracks,
      durationMs,
      cueTimes,
      this.transcoder
    );
    if (defaultAudio && !nativelySupported(defaultAudio) && canTranscode) {
      this._status('Preparing audio transcoder… (first load downloads the decoder)', 'loading');
    }
    await this.controller.start(videoTrack, defaultAudio);

    // Fonts download out-of-band (separate connections) so they don't disturb the single
    // forward media stream; subtitles render once they arrive. Fire and forget.
    this._loadFonts(player, url).catch((e) => console.warn('font loading failed', e));

    // Soft-force any forced subtitle matching the starting audio language.
    if (defaultAudio) this._applyForcedSubtitle(defaultAudio.language);

    const info = {
      videoCodec: videoTrack ? videoTrack.codec_string : null,
      audioCodec: defaultAudio ? defaultAudio.codec_string : null,
      subtitleCount: subtitleTracks.length,
      durationMs,
    };
    this._emit('ready', info);
    this._status(
      `Loaded. video=${info.videoCodec || 'none'} audio=${info.audioCodec || 'none'} ` +
        `subs=${info.subtitleCount} duration=${(durationMs / 1000).toFixed(1)}s`,
      'info'
    );
  }

  // Extract one subtitle track to WebVTT and attach it as a <track>. The WASM scan reads the
  // whole file, so this runs only when the user picks the track.
  async _loadSubtitle(player, number) {
    const vtt = await player.subtitles(BigInt(number));
    if (!vtt) return null;
    const info = this.subtitleInfo.get(number) || {};
    const blob = new Blob([vtt], { type: 'text/vtt' });
    const objectUrl = URL.createObjectURL(blob);
    this.subtitleObjectUrls.push(objectUrl);
    const track = document.createElement('track');
    track.kind = 'subtitles';
    track.label = `${info.language || '??'}${info.name ? ' — ' + info.name : ''}`;
    track.srclang = (info.language || 'und').slice(0, 3);
    track.src = objectUrl;
    this.video.appendChild(track);
    this.loadedSubs.set(number, track);
    return track;
  }

  // Turn a subtitle selection on: '' = off, an ASS track = libass, a text track = WebVTT.
  async _selectSubtitle(value) {
    // Reset both renderers first so only the chosen track is active.
    for (const tt of this.video.textTracks) tt.mode = 'disabled';
    if (this.assSubs) this.assSubs.disable();
    if (this.controller) this.controller.clearSubtitleTrack();
    if (!value || !this.activePlayer) return;

    const number = Number(value);
    const kind = this.subKindByNumber.get(number);
    if (kind === 'ass') {
      try {
        const header = this.activePlayer.subtitle_header(BigInt(number));
        await this.assSubs.enableTrack(header);
        this.controller.setSubtitleTrack(number, this.assSubs);
        this._status('ASS subtitles on (streaming).');
      } catch (e) {
        console.error(e);
        this._status('ASS subtitle error: ' + e.message);
      }
    } else if (kind === 'text') {
      let el = this.loadedSubs.get(number);
      if (!el) {
        this._status('Extracting subtitles (one-time scan)…', 'loading');
        try {
          el = await this._loadSubtitle(this.activePlayer, number);
        } catch (e) {
          console.error(e);
          this._status('Subtitle extraction failed: ' + e.message);
          return;
        }
        if (!el) {
          this._status('No subtitle cues found for that track.');
          return;
        }
        this._status('Subtitles ready.');
      }
      if (el.track) el.track.mode = 'showing';
    }
  }

  // Menu callbacks.
  _onAudioSelect(number) {
    if (this.controller) this.controller.switchAudio(number);
    const t = this.trackList.find((x) => x.number === number);
    if (t) {
      this._applyForcedSubtitle(t.language);
      this._buildChapterMenu(t.language); // re-pick chapter titles for the new audio language
    }
  }

  _onSubSelect(value) {
    this.userChoseSub = true; // explicit choice — forced-subtitle logic must not override it
    this._selectSubtitle(value);
  }

  // ---- Chapters ----

  // Pick a chapter's title in the audio language (BCP-47 then ISO-639), else the first.
  _pickChapterTitle(chapter, audioLang) {
    const displays = chapter.displays || [];
    if (audioLang) {
      const m = displays.find((d) => langMatch(d.languageBcp47 || d.language, audioLang));
      if (m) return m.text;
    }
    return (displays[0] && displays[0].text) || 'Chapter';
  }

  // ms → `m:ss` or `h:mm:ss`.
  _fmtChapterTime(ms) {
    const s = Math.floor(ms / 1000);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = String(s % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
  }

  _buildChapterMenu(audioLang) {
    const has = this.chapterList.length > 0;
    if (this.chapterMenu) {
      this.chapterMenu.setItems(
        this.chapterList.map((c) => ({
          value: String(c.startMs),
          label: `${this._fmtChapterTime(c.startMs)}  ${this._pickChapterTitle(c, audioLang)}`,
        }))
      );
      this.chapterMenu.setAvailable(has);
    }
    if (this.refs.prevChapter) this.refs.prevChapter.hidden = !has;
    if (this.refs.nextChapter) this.refs.nextChapter.hidden = !has;
    this._highlightCurrentChapter();
  }

  // Place a tick on the time slider at each chapter boundary (positioned by % of duration).
  _buildChapterMarkers(durationMs) {
    const el = this.refs.chapterMarkers;
    if (!el) return;
    el.textContent = '';
    if (!this.chapterList.length || !(durationMs > 0)) return;
    for (const c of this.chapterList) {
      if (c.startMs <= 0 || c.startMs >= durationMs) continue; // skip the implicit start
      const tick = document.createElement('div');
      tick.className = 'vjs-chapter-marker';
      tick.style.left = `${(c.startMs / durationMs) * 100}%`;
      el.appendChild(tick);
    }
  }

  _seekTo(seconds) {
    if (Number.isFinite(seconds)) this.video.currentTime = Math.max(0, seconds);
  }

  _onChapterSelect(value) {
    this._seekTo(Number(value) / 1000);
  }

  // Index of the chapter containing `ms` (largest startMs ≤ ms), or -1 before the first.
  _currentChapterIndex(ms) {
    let idx = -1;
    for (let i = 0; i < this.chapterList.length; i++) {
      if (this.chapterList[i].startMs <= ms + 1) idx = i;
      else break;
    }
    return idx;
  }

  // Highlight the current chapter in the menu as playback progresses.
  _highlightCurrentChapter() {
    if (!this.chapterMenu || !this.chapterList.length) return;
    const idx = this._currentChapterIndex(this.video.currentTime * 1000);
    this.chapterMenu.setValue(idx >= 0 ? String(this.chapterList[idx].startMs) : null);
  }

  _goToNextChapter() {
    const ms = this.video.currentTime * 1000;
    const next = this.chapterList.find((c) => c.startMs > ms + 250);
    if (next) this._seekTo(next.startMs / 1000);
  }

  _goToPrevChapter() {
    const ms = this.video.currentTime * 1000;
    const idx = this._currentChapterIndex(ms);
    if (idx <= 0) {
      this._seekTo(0);
      return;
    }
    // >3s into the current chapter restarts it; otherwise jump to the previous one.
    const restart = ms - this.chapterList[idx].startMs > 3000;
    this._seekTo(this.chapterList[restart ? idx : idx - 1].startMs / 1000);
  }

  // Fetch font attachments out-of-band (one Range request each, parallel, on separate
  // connections) so they never contend with the single forward media stream, and hand the
  // bytes to libass. baseUrl is the same URL the demuxer plays from (HTTP or a blob: URL —
  // both support Range).
  async _loadFonts(player, baseUrl) {
    const sink = this.assSubs;
    let list;
    try {
      list = JSON.parse(player.font_attachments());
    } catch {
      return;
    }
    if (!list.length) return;
    await Promise.all(
      list.map(async (f) => {
        try {
          const resp = await fetch(baseUrl, { headers: { Range: `bytes=${f.start}-${f.end}` } });
          const buf = new Uint8Array(await resp.arrayBuffer());
          // If the server ignored Range and returned the whole body (200), slice ourselves.
          const data = resp.status === 206 ? buf : buf.slice(Number(f.start), Number(f.end) + 1);
          if (sink === this.assSubs) sink.addFontData(data); // ignore if a new file loaded meanwhile
        } catch (e) {
          console.warn(`font "${f.name}" fetch failed`, e);
        }
      })
    );
  }

  // Soft-force a forced subtitle for `audioLang` (foreign signs/songs) — but only if the user
  // hasn't made their own subtitle choice.
  _applyForcedSubtitle(audioLang) {
    if (this.userChoseSub) return;
    const forced = this.trackList.find(
      (t) =>
        t.type === 'subtitle' &&
        t.forced &&
        this.subKindByNumber.has(t.number) &&
        langMatch(t.language, audioLang)
    );
    if (!forced) return;
    if (this.subsMenu) this.subsMenu.setValue(String(forced.number));
    this._selectSubtitle(String(forced.number)); // programmatic — keep userChoseSub false
  }

  _reportTracks(tracks) {
    const lines = tracks.map(
      (t) =>
        `  #${t.number} ${t.type} ${t.codec_id} ${t.mime ? `(${t.mime})` : '(not muxable)'} lang=${t.language}`
    );
    console.log('Tracks:\n' + lines.join('\n'));
  }

  // Tear down the current playback session (called between loads and on destroy).
  _teardownSession() {
    if (this.controller) {
      try {
        this.video.removeAttribute('src');
        this.video.load();
      } catch (_) {}
      this.controller = null;
    }
    this.subtitleObjectUrls.forEach((u) => URL.revokeObjectURL(u));
    this.subtitleObjectUrls = [];
    this.loadedSubs.clear();
    this.subtitleInfo.clear();
    this.subKindByNumber.clear();
    this.userChoseSub = false;
    if (this.assSubs) {
      this.assSubs.destroy();
      this.assSubs = null;
    }
    if (this.transcoder) {
      this.transcoder.destroy();
      this.transcoder = null;
    }
    this.activePlayer = null;
    for (const t of [...this.video.querySelectorAll('track')]) t.remove();
    if (this.audioMenu) {
      this.audioMenu.setItems([]);
      this.audioMenu.setAvailable(false);
    }
    if (this.subsMenu) {
      this.subsMenu.setItems([]);
      this.subsMenu.setAvailable(false);
    }
    if (this.chapterMenu) {
      this.chapterMenu.setItems([]);
      this.chapterMenu.setAvailable(false);
    }
    this.chapterList = [];
    if (this.refs.chapterMarkers) this.refs.chapterMarkers.textContent = '';
    this._setTitle('');
  }

  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    this._teardownSession();
    if (this.refs.prevChapter) this.refs.prevChapter.removeEventListener('click', this._onPrevChapter);
    if (this.refs.nextChapter) this.refs.nextChapter.removeEventListener('click', this._onNextChapter);
    this.video.removeEventListener('timeupdate', this._onTimeUpdate);
    this.audioMenu?.destroy();
    this.subsMenu?.destroy();
    this.chapterMenu?.destroy();
    this.container.innerHTML = '';
    this._listeners = { status: [], error: [], ready: [], tracks: [] };
  }
}
