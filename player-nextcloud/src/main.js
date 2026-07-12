// Registers the MKV/MKA player as a Nextcloud Viewer handler. This script is injected on the
// Files app, public shares and wherever the Viewer loads (see LoadViewerListener.php), after the
// Viewer's own scripts — so window.OCA.Viewer exists by the time we register.
import PlayerView from './views/player-view.js';
import { installRangeFetchFix } from './range-fetch-fix.js';
import './player-view.css';

// Must run before any playback so the WASM's byte-range reads keep their Range header.
installRangeFetchFix();

const HANDLER = {
	id: 'matroskaplayer',
	// Matroska video (.mkv → video/x-matroska) and audio (.mka → audio/x-matroska). The Viewer's
	// built-in video handler *aliases* video/x-matroska → video/webm to try native <video>
	// playback; registering the mime directly here takes precedence so our WASM player handles it
	// instead (the Viewer logs a benign "already registered" notice about the losing alias).
	// (fileExtensions is a forward-compat hint; the Viewer in NC ≤30 matches by MIME only, and
	// .mka has no default MIME mapping — see README.)
	mimes: ['video/x-matroska', 'audio/x-matroska'],
	fileExtensions: ['mkv', 'mka'],
	component: PlayerView,
};

function register() {
	if (!window.OCA?.Viewer?.registerHandler) {
		return false;
	}
	// Guard against double registration. The script can be injected by more than one load event
	// (Files' LoadAdditionalScriptsEvent, LoadViewer, public-share render), so use a global flag
	// that survives separate module evaluations, plus the Viewer's own handler list.
	if (window.__matroskaplayerRegistered || window.OCA.Viewer.availableHandlers?.some((h) => h.id === HANDLER.id)) {
		window.__matroskaplayerRegistered = true;
		return true;
	}
	window.OCA.Viewer.registerHandler(HANDLER);
	window.__matroskaplayerRegistered = true;
	return true;
}

// Our script depends on the Viewer's, so OCA.Viewer is normally ready synchronously; fall back to
// DOMContentLoaded, then a short poll, in case ordering differs on some pages (e.g. public shares).
if (!register()) {
	document.addEventListener('DOMContentLoaded', () => {
		if (register()) {
			return;
		}
		let tries = 0;
		const t = setInterval(() => {
			if (register() || ++tries > 20) {
				clearInterval(t);
			}
		}, 250);
	});
}
