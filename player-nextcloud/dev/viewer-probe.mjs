// Headless integration probe: log into the dev Nextcloud, open a file in the Viewer via
// OCA.Viewer.open(), and report whether our handler mounted + the player ran. Headless chromium
// can't *decode* video, so we assert up to remux/MSE. Usage: node dev/viewer-probe.mjs [path]
import { chromium } from 'playwright-core'

const BASE = process.env.BASE || 'http://localhost:8080'
const PATH = process.argv[2] || '/test-h264.mkv'
const WAIT = Number(process.env.WAIT || 15000)

const browser = await chromium.launch({
	executablePath: '/usr/bin/chromium',
	args: ['--no-sandbox', '--autoplay-policy=no-user-gesture-required'],
})
const ctx = await browser.newContext({ ignoreHTTPSErrors: true })
const page = await ctx.newPage()

const log = []
const net = []
page.on('console', (m) => log.push(`[${m.type()}] ${m.text()}`.slice(0, 300)))
page.on('pageerror', (e) => log.push('[pageerror] ' + e.message))
page.on('requestfailed', (r) => {
	const u = r.url()
	if (/dav|\.mkv|\.mka|\.wasm|download/i.test(u)) net.push(`FAILED ${r.request().method()} ${u.replace(BASE, '')} — ${r.failure()?.errorText}`)
})
page.on('response', (r) => {
	const u = r.url()
	if (/dav\/files|download|\.mkv|\.mka|\.wasm/i.test(u)) {
		const h = r.request().headers()
		const extra = /\.mkv|\.mka/i.test(u) ? ` [range=${h.range || 'NONE'}] [cookie=${h.cookie ? 'yes' : 'NO'}] [mode/dest=${h['sec-fetch-mode'] || '?'}/${h['sec-fetch-dest'] || '?'}]` : ''
		net.push(`${r.status()} ${r.request().method()} ${u.replace(BASE, '').slice(0, 55)}${extra}`)
	}
})

await page.goto(`${BASE}/login`, { waitUntil: 'networkidle', timeout: 30000 })
await page.fill('input[name="user"]', 'admin')
await page.fill('input[name="password"]', 'admin')
await page.click('button[type="submit"], input[type="submit"]')
await page.waitForLoadState('networkidle', { timeout: 30000 })

await page.goto(`${BASE}/apps/files/`, { waitUntil: 'networkidle', timeout: 30000 })
await page.waitForFunction(() => !!(window.OCA?.Viewer?.open), { timeout: 20000 })
const handlerRegistered = await page.evaluate(() => (window.OCA.Viewer.availableHandlers || []).some((h) => h.id === 'matroskaplayer'))

await page.evaluate((p) => window.OCA.Viewer.open({ path: p }), PATH)

let mounted = false
try { await page.waitForSelector('.matroskaplayer-stage', { timeout: 15000 }); mounted = true } catch { /**/ }
await page.waitForTimeout(WAIT)

const handed = await page.evaluate(() => window.__mkv || null)
const state = await page.evaluate(() => {
	const v = document.querySelector('.matroskaplayer-stage video')
	return {
		hasStage: !!document.querySelector('.matroskaplayer-stage'),
		hasVideoEl: !!v,
		video: v ? { readyState: v.readyState, currentTime: +v.currentTime.toFixed(2), duration: v.duration, w: v.videoWidth, err: v.error?.code ?? null } : null,
		status: document.querySelector('.matroskaplayer-status')?.textContent?.trim() || null,
	}
})

console.log('handlerRegistered:', handlerRegistered, '| mounted:', mounted)
console.log('handed:', JSON.stringify(handed))
console.log('state:', JSON.stringify(state))
console.log('\n--- media/net ---\n' + net.join('\n'))
console.log('\n--- console (player/status/error lines) ---\n' + log.filter((l) => /video=|audio=|duration=|error|fail|cannot|status|preflight|206|remux|track/i.test(l)).slice(0, 25).join('\n'))

await browser.close()
