// Entry for the admin settings page. Mounts the license settings Vue 3 app into the mount point
// rendered by templates/admin-settings.php. Kept separate from the Viewer handler (src/main.js),
// which runs on Vue 2 — this page bundles its own Vue 3 runtime and never coexists with it.
import { createApp } from 'vue'
import AdminSettings from './AdminSettings.vue'

const el = document.getElementById('matroskaplayer-admin-settings')
if (el) {
	createApp(AdminSettings).mount(el)
}
