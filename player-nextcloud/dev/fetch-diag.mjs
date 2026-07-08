// Isolate browser-fetch behaviour for the DAV URL from inside the logged-in NC page context.
import { chromium } from 'playwright-core'
const BASE = 'http://localhost:8080'
const SRC = BASE + '/remote.php/dav/files/admin/bbb.mkv'

const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] })
const page = await (await browser.newContext()).newPage()
await page.goto(`${BASE}/login`, { waitUntil: 'networkidle' })
await page.fill('input[name="user"]', 'admin')
await page.fill('input[name="password"]', 'admin')
await page.click('button[type="submit"], input[type="submit"]')
await page.waitForLoadState('networkidle')
await page.goto(`${BASE}/apps/files/`, { waitUntil: 'networkidle' })

const out = await page.evaluate(async (src) => {
	const probe = async (opts, label) => {
		try {
			const r = await fetch(src, opts)
			const buf = new Uint8Array(await r.arrayBuffer())
			return { label, status: r.status, contentRange: r.headers.get('content-range'), len: buf.length, hex: [...buf.slice(0, 4)].map((x) => x.toString(16).padStart(2, '0')).join(' ') }
		} catch (e) {
			return { label, error: String(e) }
		}
	}
	return [
		await probe({ headers: { Range: 'bytes=0-15' } }, 'range 0-15'),
		await probe({ headers: { Range: 'bytes=-262144' } }, 'suffix -262144'),
		await probe({ headers: { Range: 'bytes=0-32767' }, credentials: 'omit' }, 'range 0-32767 credentials=omit'),
		await probe({}, 'no range'),
	]
}, SRC)

console.log(JSON.stringify(out, null, 2))
await browser.close()
