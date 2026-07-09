// The Viewer handler component. Nextcloud's Viewer (v3, in NC ≤30) runs on **Vue 2** and
// instantiates registered components with its own runtime, so this is a plain Vue-2 options
// object with a `render(h)` function — NOT a compiled `.vue` SFC (which would bundle a Vue 3
// runtime whose vnodes the Viewer's Vue 2 can't patch). The Viewer mixes its own mixin in at
// registration, giving us the props `active`, `filename`, `source`, `mime`, … and `doneLoading()`.
//
// The heavy player library (WASM remuxer + workers) is loaded lazily on first open so it stays
// out of the always-injected entry bundle.
import { loadState } from '@nextcloud/initial-state'
import { generateFilePath } from '@nextcloud/router'

/** Read the app config seeded via initial state (transcode/debug/external flags). */
function loadConfig() {
	try {
		return loadState('mkvplayer', 'config') || {}
	} catch (e) {
		return {}
	}
}

/**
 * Where to load the ffmpeg core from: the admin's external URLs if opted in, otherwise the
 * royalty-free core bundled with the app and served same-origin (offline).
 */
function resolveFfmpeg(config) {
	if (config.external && config.external.coreURL && config.external.wasmURL) {
		return config.external
	}
	return {
		coreURL: generateFilePath('mkvplayer', '', 'ffmpeg/ffmpeg-core.js'),
		wasmURL: generateFilePath('mkvplayer', '', 'ffmpeg/ffmpeg-core.wasm'),
	}
}

export default {
	name: 'MkvPlayerView',

	data() {
		return {
			config: loadConfig(),
			player: null,
			statusMessage: '',
			started: false,
		}
	},

	render(h) {
		const children = [h('div', { ref: 'stage', class: 'mkvplayer-stage' })]
		// The status/loading/error overlay is only rendered when the admin enabled debugging.
		if (this.config.debug && this.statusMessage) {
			children.push(h('div', { class: 'mkvplayer-status' }, this.statusMessage))
		}
		return h('div', { class: 'mkvplayer-outer' }, children)
	},

	mounted() {
		// The Viewer only shows the active file; start playback for it.
		if (this.active) {
			this.start()
		}
	},

	watch: {
		active(isActive) {
			if (isActive) {
				this.start()
			}
		},
	},

	beforeDestroy() {
		this.teardown()
	},

	methods: {
		async start() {
			if (this.started) {
				return
			}
			this.started = true
			this.statusMessage = 'Loading player…'

			const cfg = this.config

			try {
				// Lazy chunk: pulls in the remuxer WASM + workers only when a file is opened.
				const [{ createPlayer }] = await Promise.all([
					import('mkv-player-ui'),
					import('mkv-player-ui/style.css'),
				])
				if (this._teardown) {
					return
				}

				this.player = createPlayer(this.$refs.stage, {
					controls: 'full',
					// Transcoding uses the bundled same-origin core by default (or the admin's
					// external URLs when opted in); resolveFfmpeg() picks between them.
					transcode: cfg.transcodeEnabled ? 'auto' : false,
					ffmpeg: resolveFfmpeg(cfg),
					onStatus: (msg) => {
						this.statusMessage = msg
					},
					onReady: () => {
						this.statusMessage = ''
						// Drop the Viewer's loading spinner (method from the injected mixin).
						if (typeof this.doneLoading === 'function') {
							this.doneLoading()
						}
					},
					onError: (err) => {
						this.statusMessage = 'Error: ' + err.message
						this.$emit('error', err)
					},
				})

				// The Viewer hands us `source`: a same-origin URL to the file, so the player's
				// ranged fetch is authenticated by the session cookie (see resolveSource).
				this.player.load(this.resolveSource()).catch(() => {})
			} catch (err) {
				this.statusMessage = 'Error: ' + err.message
			}
		},

		resolveSource() {
			return this.source
		},

		teardown() {
			this._teardown = true
			if (this.player) {
				this.player.destroy()
				this.player = null
			}
		},
	},
}
