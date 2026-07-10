// Builds the video.js v10 (@videojs/html) player markup into a container, including only
// the controls the caller enabled. This is what makes controls addable/removable: instead
// of the hand-written control bar the demo apps used to keep in their index.html, the
// library assembles it here from template-string fragments — one per control — and returns
// scoped element references (via `data-ref`, not global ids, so multiple players coexist).
//
// The markup mirrors the ejected skin the apps shipped; only a control's *presence* varies.

const CONTROL_KEYS = [
  'play',
  'seek',
  'chapterSkip',
  'timeSlider',
  'chapterMarkers',
  'chapters',
  'audio',
  'subtitles',
  'volume',
  'fullscreen',
  'hotkeys',
  'gestures',
];

// Presets. `full` = the demo apps' current control bar; `minimal` = transport + volume +
// fullscreen only; `none` = a bare video (no control bar, no hotkeys/gestures).
const PRESETS = {
  full: {
    play: true, seek: true, chapterSkip: true, timeSlider: true, chapterMarkers: true,
    chapters: true, audio: true, subtitles: true, volume: true, fullscreen: true,
    hotkeys: true, gestures: true,
  },
  minimal: {
    play: true, seek: false, chapterSkip: false, timeSlider: true, chapterMarkers: false,
    chapters: false, audio: false, subtitles: false, volume: true, fullscreen: true,
    hotkeys: true, gestures: true,
  },
  none: {
    play: false, seek: false, chapterSkip: false, timeSlider: false, chapterMarkers: false,
    chapters: false, audio: false, subtitles: false, volume: false, fullscreen: false,
    hotkeys: false, gestures: false,
  },
};

// Resolve a `controls` option (a preset name, or an object with an optional `preset` plus
// per-control boolean overrides) into a full flag set.
export function resolveControls(controls) {
  if (controls == null) controls = 'full';
  let presetName = 'full';
  let overrides = {};
  if (typeof controls === 'string') {
    presetName = controls;
  } else if (typeof controls === 'object') {
    presetName = controls.preset || 'full';
    overrides = controls;
  }
  const base = PRESETS[presetName] || PRESETS.full;
  const flags = {};
  for (const k of CONTROL_KEYS) {
    flags[k] = k in overrides ? !!overrides[k] : !!base[k];
  }
  return flags;
}

let seq = 0; // per-page instance counter → unique ids for commandfor/popover wiring

// ---- markup fragments (each returns a string, or '' when the control is disabled) ----

const playFrag = (uid) => `
  <media-play-button commandfor="play-tooltip-${uid}" class="media-button media-button--subtle media-button--icon media-button--play">
    <media-icon name="restart" class="media-icon media-icon--restart"></media-icon>
    <media-icon name="play" class="media-icon media-icon--play"></media-icon>
    <media-icon name="pause" class="media-icon media-icon--pause"></media-icon>
  </media-play-button>
  <media-tooltip id="play-tooltip-${uid}" side="top" class="media-surface media-tooltip"></media-tooltip>`;

const prevChapterFrag = () => `
  <button type="button" data-ref="prevChapter" class="media-button media-button--subtle vjs-chapter-skip" title="Previous chapter" aria-label="Previous chapter" hidden>⏮</button>`;

const nextChapterFrag = () => `
  <button type="button" data-ref="nextChapter" class="media-button media-button--subtle vjs-chapter-skip" title="Next chapter" aria-label="Next chapter" hidden>⏭</button>`;

const seekFrag = (uid) => `
  <media-seek-button commandfor="seek-backward-tooltip-${uid}" seconds="-10" class="media-button media-button--subtle media-button--icon media-button--seek">
    <span class="media-icon__container">
      <media-icon name="seek" class="media-icon media-icon--flipped"></media-icon>
      <span class="media-icon__label">10</span>
    </span>
  </media-seek-button>
  <media-tooltip id="seek-backward-tooltip-${uid}" side="top" class="media-surface media-tooltip"></media-tooltip>
  <media-seek-button commandfor="seek-forward-tooltip-${uid}" seconds="10" class="media-button media-button--subtle media-button--icon media-button--seek">
    <span class="media-icon__container">
      <media-icon name="seek" class="media-icon"></media-icon>
      <span class="media-icon__label">10</span>
    </span>
  </media-seek-button>
  <media-tooltip id="seek-forward-tooltip-${uid}" side="top" class="media-surface media-tooltip"></media-tooltip>`;

const timeSliderFrag = (flags) => `
  <div class="media-time-controls">
    <media-time type="current" class="media-time"></media-time>
    <media-time-slider class="media-slider">
      <media-slider-track class="media-slider__track">
        <media-slider-fill class="media-slider__fill"></media-slider-fill>
        <media-slider-buffer class="media-slider__buffer"></media-slider-buffer>
      </media-slider-track>
      ${flags.chapterMarkers ? '<div class="vjs-chapter-markers" data-ref="chapterMarkers"></div>' : ''}
      <media-slider-thumb class="media-slider__thumb"></media-slider-thumb>
    </media-time-slider>
    <media-time type="duration" class="media-time"></media-time>
  </div>`;

const menuFrag = (ref, title, label) => `
  <div class="vjs-menu">
    <button type="button" data-ref="${ref}Trigger" class="media-button media-button--subtle vjs-menu__trigger" aria-haspopup="true" aria-expanded="false" title="${title}" hidden>${label}</button>
    <div data-ref="${ref}Menu" class="media-surface vjs-menu__popup" role="menu" hidden></div>
  </div>`;

const volumeFrag = (uid) => `
  <media-mute-button commandfor="video-volume-popover-${uid}" class="media-button media-button--subtle media-button--icon media-button--mute">
    <media-icon name="volume-off" class="media-icon media-icon--volume-off"></media-icon>
    <media-icon name="volume-low" class="media-icon media-icon--volume-low"></media-icon>
    <media-icon name="volume-high" class="media-icon media-icon--volume-high"></media-icon>
  </media-mute-button>
  <media-popover id="video-volume-popover-${uid}" open-on-hover delay="200" close-delay="100" side="top" class="media-surface media-popover media-popover--volume">
    <media-volume-slider class="media-slider" orientation="vertical" thumb-alignment="edge">
      <media-slider-track class="media-slider__track">
        <media-slider-fill class="media-slider__fill"></media-slider-fill>
      </media-slider-track>
      <media-slider-thumb class="media-slider__thumb media-slider__thumb--persistent"></media-slider-thumb>
    </media-volume-slider>
  </media-popover>`;

const fullscreenFrag = (uid) => `
  <media-fullscreen-button commandfor="fullscreen-tooltip-${uid}" class="media-button media-button--subtle media-button--icon media-button--fullscreen">
    <media-icon name="fullscreen-enter" class="media-icon media-icon--fullscreen-enter"></media-icon>
    <media-icon name="fullscreen-exit" class="media-icon media-icon--fullscreen-exit"></media-icon>
  </media-fullscreen-button>
  <media-tooltip id="fullscreen-tooltip-${uid}" side="top" class="media-surface media-tooltip"></media-tooltip>`;

const hotkeysFrag = () => `
  <media-hotkey keys="Space" action="togglePaused"></media-hotkey>
  <media-hotkey keys="k" action="togglePaused"></media-hotkey>
  <media-hotkey keys="m" action="toggleMuted"></media-hotkey>
  <media-hotkey keys="f" action="toggleFullscreen"></media-hotkey>
  <media-hotkey keys="ArrowRight" action="seekStep" value="5"></media-hotkey>
  <media-hotkey keys="ArrowLeft" action="seekStep" value="-5"></media-hotkey>
  <media-hotkey keys="ArrowUp" action="volumeStep" value="0.05"></media-hotkey>
  <media-hotkey keys="ArrowDown" action="volumeStep" value="-0.05"></media-hotkey>`;

const gesturesFrag = () => `
  <media-gesture type="tap" action="togglePaused" pointer="mouse" region="center"></media-gesture>
  <media-gesture type="doubletap" action="toggleFullscreen" region="center"></media-gesture>`;

/**
 * Build the player DOM into `container`, keeping only the controls in `controlsConfig`.
 * Returns `{ video, titleBar, watermark, audioTrigger, audioMenu, subsTrigger, subsMenu,
 * chaptersTrigger, chaptersMenu, prevChapter, nextChapter, chapterMarkers }`; entries for
 * disabled controls are `null`. The `<video>` always stays nested in `<media-container>` so
 * the JASSUB subtitle overlay (inserted after the video) positions correctly even with no
 * controls. When `watermarkConfig` is truthy a bottom-right `.mkv-watermark` element is added
 * (populated by the orchestrator); it lives alongside the title bar so it shares the video's
 * containing block in both layout modes (see style.css).
 */
export function buildControlBar(container, controlsConfig, watermarkConfig) {
  const flags = resolveControls(controlsConfig);
  const uid = `mp${++seq}`;

  // First button group: play + chapter-skip + seek. Chapter-skip flanks the seek buttons,
  // matching the original layout.
  const group1 = [
    flags.play ? playFrag(uid) : '',
    flags.chapterSkip ? prevChapterFrag() : '',
    flags.seek ? seekFrag(uid) : '',
    flags.chapterSkip ? nextChapterFrag() : '',
  ].join('');

  // Second button group: track menus + volume + fullscreen.
  const group2 = [
    flags.chapters ? menuFrag('chapters', 'Chapters', 'Chapters') : '',
    flags.audio ? menuFrag('audio', 'Audio track', 'Audio') : '',
    flags.subtitles ? menuFrag('subs', 'Subtitles', 'Subs') : '',
    flags.volume ? volumeFrag(uid) : '',
    flags.fullscreen ? fullscreenFrag(uid) : '',
  ].join('');

  const hasControls =
    flags.play || flags.seek || flags.chapterSkip || flags.timeSlider ||
    flags.chapters || flags.audio || flags.subtitles || flags.volume || flags.fullscreen;

  // `data-interactive` marks the whole control bar as an interactive surface so the tap
  // gesture (togglePaused, region "center" = full surface) never fires when the user clicks
  // control-bar chrome. Native buttons and the ARIA-role'd menu already match the gesture's
  // interactive selector, but the bar's non-button parts (time readout, gaps, group wrappers)
  // and our custom menu popups did not — so clicks there "fell through" and toggled playback.
  const controlsMarkup = hasControls
    ? `<media-controls data-interactive class="media-surface media-controls">
        <media-tooltip-group>
          ${group1 ? `<div class="media-button-group">${group1}</div>` : ''}
          ${flags.timeSlider ? timeSliderFrag(flags) : ''}
          ${group2 ? `<div class="media-button-group">${group2}</div>` : ''}
        </media-tooltip-group>
      </media-controls>`
    : '';

  // A title bar across the top of the player. Populated by the player orchestrator and shown
  // only when there are controls — its visibility mirrors the control bar's (see style.css).
  const titleBarMarkup = hasControls
    ? `<div class="mkv-title-bar" data-ref="titleBar" hidden></div>`
    : '';

  // Bottom-right watermark. Always visible while shown, but its opacity/position react to the
  // control-bar visibility purely in CSS (see style.css) — no per-frame JS. Content is filled in
  // by the orchestrator (_setWatermark), so it starts hidden until then.
  const watermarkMarkup = watermarkConfig
    ? `<div class="mkv-watermark" data-ref="watermark" hidden></div>`
    : '';

  // `dock: 'below'` renders the bar *under* the video instead of overlaying it (see style.css).
  // The container gets a mode class and, in docked mode, the video + its overlays are wrapped in
  // an `.mkv-stage` so they position over the picture only while the bar flows beneath it.
  const dock =
    controlsConfig && typeof controlsConfig === 'object' && controlsConfig.dock === 'below'
      ? 'below'
      : 'overlay';
  const containerClass =
    'media-default-skin media-default-skin--video' +
    (dock === 'below' ? ' media-default-skin--dock-below' : '');

  const videoEl = `<video data-ref="video" playsinline crossorigin="anonymous"></video>`;
  const bufferingEl = `<media-buffering-indicator class="media-buffering-indicator">
          <div class="media-surface">
            <media-icon name="spinner" class="media-icon"></media-icon>
          </div>
        </media-buffering-indicator>`;
  const overlayEl = `<div class="media-overlay"></div>`;
  const hotkeysEl = flags.hotkeys ? hotkeysFrag() : '';
  const gesturesEl = flags.gestures ? gesturesFrag() : '';

  // Docked: [stage: video + overlays] then the control bar beneath it.
  // Overlay (default): the original flat order (controls before the scrim, so the skin's
  // `.media-controls[data-visible] ~ .media-overlay` sibling selector keeps working).
  const containerInner =
    dock === 'below'
      ? `<div class="mkv-stage">
          ${videoEl}
          ${titleBarMarkup}
          ${watermarkMarkup}
          ${bufferingEl}
          ${overlayEl}
          ${gesturesEl}
        </div>
        ${controlsMarkup}
        ${hotkeysEl}`
      : `${videoEl}
        ${titleBarMarkup}
        ${watermarkMarkup}
        ${bufferingEl}
        ${controlsMarkup}
        ${overlayEl}
        ${hotkeysEl}
        ${gesturesEl}`;

  container.innerHTML = `
    <video-player>
      <media-container class="${containerClass}">
        ${containerInner}
      </media-container>
    </video-player>`;

  const q = (ref) => container.querySelector(`[data-ref="${ref}"]`);
  return {
    video: q('video'),
    titleBar: q('titleBar'),
    watermark: q('watermark'),
    audioTrigger: q('audioTrigger'),
    audioMenu: q('audioMenu'),
    subsTrigger: q('subsTrigger'),
    subsMenu: q('subsMenu'),
    chaptersTrigger: q('chaptersTrigger'),
    chaptersMenu: q('chaptersMenu'),
    prevChapter: q('prevChapter'),
    nextChapter: q('nextChapter'),
    chapterMarkers: q('chapterMarkers'),
  };
}
