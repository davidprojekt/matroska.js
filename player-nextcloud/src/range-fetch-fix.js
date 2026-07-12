// Workaround for a Nextcloud fetch wrapper that breaks the player's byte-range reads.
//
// The @matroska-js/remux WASM issues range reads as `fetch(new Request(url, { headers: { Range } }))`.
// Nextcloud replaces `window.fetch` with a wrapper that augments requests by building an `init`
// object and re-invoking the inner fetch as `fetch(request, init)`. Per the Fetch spec, when
// both a Request and an `init.headers` are supplied, the init headers replace the Request's — and
// NC's init doesn't carry the Range header, so it is dropped. Nextcloud's WebDAV then returns the
// whole file with `200` instead of a `206` partial, and every WASM read path bails on a non-206
// response, so the remuxer sees a seekless/empty stream and reports zero tracks.
//
// A *pristine* native `fetch` preserves the Range header on a Request object, so we re-route any
// range-bearing request through a native fetch taken from a hidden same-origin iframe (untouched
// by NC's wrapper). The Range value lives on the Request's own headers, so we forward those. The
// media read is a same-origin GET authenticated by the session cookie; NC's extra request headers
// (e.g. CSRF token) aren't needed for it.
//
// The iframe's `Response` belongs to the iframe realm, and the WASM traps ("unreachable") when
// reading such a cross-realm response body's stream. So we read the body here and hand back a
// fresh same-realm `Response` over the buffered bytes (each request the WASM makes is bounded — a
// small range or one forward SEGMENT — so buffering it is fine).
//
// TODO(upstream): ideally the @matroska-js/remux fetch layer would pass Range via `fetch(url, { headers })`
// (which NC's wrapper preserves); this shim lets the Nextcloud app work with the library as-is.
let pristineFetch

function getPristineFetch() {
	if (pristineFetch) {
		return pristineFetch
	}
	const iframe = document.createElement('iframe')
	iframe.setAttribute('aria-hidden', 'true')
	iframe.style.display = 'none'
	document.body.appendChild(iframe)
	// about:blank inherits our origin, so its fetch is same-origin (cookies flow) but is the
	// untouched native implementation.
	pristineFetch = iframe.contentWindow.fetch.bind(iframe.contentWindow)
	return pristineFetch
}

export function installRangeFetchFix() {
	if (typeof window === 'undefined' || window.__mkvRangeFetchFix) {
		return
	}
	window.__mkvRangeFetchFix = true

	const wrapped = window.fetch.bind(window)
	window.fetch = (input, init) => {
		if (input instanceof Request && input.headers.get('range')) {
			return rangeFetch(input)
		}
		return wrapped(input, init)
	}
}

/** Re-issue a range-bearing request via the pristine native fetch, returning a same-realm Response. */
async function rangeFetch(request) {
	const res = await getPristineFetch()(request.url, {
		method: request.method,
		headers: Object.fromEntries(request.headers.entries()),
		credentials: request.credentials,
		mode: request.mode,
	})
	const body = await res.arrayBuffer()
	const headers = new Headers()
	res.headers.forEach((value, key) => headers.set(key, value))
	return new Response(body, { status: res.status, statusText: res.statusText, headers })
}
