<!--
  License settings for the MKV Player admin section. A masked field for the Ed25519 license key,
  a Save action that persists + validates it server-side (the raw key never reaches end users),
  a live valid/invalid status, and a "Buy" link button that opens the purchase page with this
  instance's id already appended. Styled with Nextcloud's global settings classes (no
  @nextcloud/vue dependency) plus a little scoped CSS.
-->
<template>
	<div id="mkvplayer-license" class="section">
		<h2>{{ 'MKV Player license' }}</h2>
		<p class="settings-hint">
			A valid license removes the player watermark. The key is validated on this server; it is
			never sent to viewers.
		</p>

		<div class="mkvplayer-license__field">
			<input
				id="mkvplayer-license-key"
				v-model="key"
				type="password"
				autocomplete="off"
				spellcheck="false"
				:placeholder="hasKey ? '•••••••• (a key is already stored)' : 'Paste your license key'"
				@keyup.enter="save" />
			<button
				class="primary"
				:disabled="saving || key.trim() === ''"
				@click="save">
				{{ saving ? 'Saving…' : 'Save & validate' }}
			</button>
		</div>

		<p v-if="statusText" class="mkvplayer-license__status" :class="valid ? 'is-valid' : 'is-invalid'">
			{{ statusText }}
		</p>

		<p class="mkvplayer-license__buy">
			<a class="button" :href="buyUrl" target="_blank" rel="noopener noreferrer">
				Buy a license
			</a>
		</p>
	</div>
</template>

<script>
import { loadState } from '@nextcloud/initial-state'
import { generateUrl } from '@nextcloud/router'

/** Read the admin-only license state seeded by LicenseAdminSettings (never the raw key). */
function loadLicenseState() {
	try {
		return loadState('mkvplayer', 'license') || {}
	} catch (e) {
		return {}
	}
}

export default {
	name: 'AdminSettings',

	data() {
		const s = loadLicenseState()
		return {
			key: '',
			hasKey: !!s.hasKey,
			valid: !!s.valid,
			email: s.email || null,
			buyUrl: s.buyUrl || '#',
			saving: false,
			saved: false,
		}
	},

	computed: {
		statusText() {
			if (this.saving) {
				return ''
			}
			if (this.valid) {
				return this.email ? `Valid license for ${this.email}.` : 'Valid license.'
			}
			if (this.saved) {
				return 'Invalid license key for this instance.'
			}
			if (this.hasKey) {
				return 'A stored key is not valid for this instance.'
			}
			return ''
		},
	},

	methods: {
		async save() {
			if (this.key.trim() === '') {
				return
			}
			this.saving = true
			try {
				const resp = await fetch(generateUrl('/apps/mkvplayer/settings/license'), {
					method: 'POST',
					headers: {
						'Content-Type': 'application/json',
						requesttoken: (window.OC && window.OC.requestToken) || '',
					},
					body: JSON.stringify({ key: this.key }),
				})
				const data = await resp.json()
				this.valid = !!data.valid
				this.email = data.email || null
				this.hasKey = this.hasKey || this.key.trim() !== ''
			} catch (e) {
				this.valid = false
				this.email = null
			} finally {
				this.saved = true
				this.saving = false
			}
		},
	},
}
</script>

<style scoped>
.mkvplayer-license__field {
	display: flex;
	gap: 0.5rem;
	align-items: center;
	flex-wrap: wrap;
	max-width: 40rem;
	margin-top: 0.5rem;
}
.mkvplayer-license__field input[type='password'] {
	flex: 1 1 20rem;
}
.mkvplayer-license__status {
	font-weight: 600;
}
.mkvplayer-license__status.is-valid {
	color: var(--color-success, #2d7b41);
}
.mkvplayer-license__status.is-invalid {
	color: var(--color-error, #c9302c);
}
.mkvplayer-license__buy .button {
	display: inline-block;
}
</style>
