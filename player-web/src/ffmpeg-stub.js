// Build-time replacement for @ffmpeg/* in `TRANSCODE=off` builds (see vite.config.js).
// audioTranscoder.js is dead code in that build, but Vite would still transform it and
// emit the ffmpeg worker as a side effect; aliasing the @ffmpeg packages here keeps the
// bundle genuinely ffmpeg-free. The named exports exist only to satisfy the bundler.
export const FFmpeg = undefined;
export const toBlobURL = undefined;
