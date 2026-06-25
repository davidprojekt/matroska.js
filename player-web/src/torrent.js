// WebTorrent integration: add a torrent (magnet link or .torrent file), wait for
// metadata, and expose its files. The chosen file is served over HTTP by a service
// worker (client.createServer), whose stream URL supports byte-range (206) requests —
// exactly what MatroskaPlayer.open() needs, so it can be dropped straight into the
// existing player as a Video URL.
//
// We import the prebuilt browser bundle rather than the package source: it inlines the
// Buffer/process polyfills WebTorrent expects, sidestepping node-shim issues under Vite.
//
// Browser caveat: WebTorrent in the browser can only reach peers over WebRTC and WSS
// trackers (plus web seeds) — it cannot talk to plain TCP/uTP BitTorrent peers. A magnet
// for a "normal" torrent will sit forever at "waiting for metadata" because no WebRTC
// peers exist. Test with a WebRTC-seeded torrent (e.g. the Sintel demo magnet).
import WebTorrent from 'webtorrent/dist/webtorrent.min.js';

let client = null;
let serverReady = null;
let currentTorrent = null;

/** Lazily create the singleton client + register the stream-serving service worker. */
async function ensureClient() {
  if (client) {
    await serverReady;
    return client;
  }
  if (!('serviceWorker' in navigator)) {
    throw new Error('Service workers are unavailable (needs HTTPS or localhost) — required for WebTorrent streaming.');
  }
  client = new WebTorrent();
  serverReady = (async () => {
    const controller = await navigator.serviceWorker.register('/sw.min.js', { scope: '/' });
    await navigator.serviceWorker.ready;
    client.createServer({ controller });
  })();
  await serverReady;
  return client;
}

/** Remove the previously added torrent (if any) so we don't keep seeding/downloading it. */
function removeCurrent(c) {
  return new Promise((resolve) => {
    if (!currentTorrent) return resolve();
    const prev = currentTorrent;
    currentTorrent = null;
    c.remove(prev.infoHash, {}, () => resolve());
  });
}

/**
 * Add a torrent and resolve once its metadata (file list) is available.
 * @param source  magnet URI string, or a .torrent File/Blob
 * @param onProgress  optional (message) => void for status updates while waiting
 * @returns the WebTorrent torrent (with .files populated)
 */
export async function addTorrent(source, onProgress = () => {}) {
  const c = await ensureClient();
  await removeCurrent(c);

  // A File/Blob needs to be read into bytes; magnet strings pass through as-is.
  let torrentId = source;
  if (typeof source !== 'string') {
    onProgress('Reading .torrent file…');
    torrentId = new Uint8Array(await source.arrayBuffer());
  }

  onProgress('Connecting to peers and fetching metadata…');
  return new Promise((resolve, reject) => {
    let settled = false;
    const torrent = c.add(torrentId, (t) => {
      settled = true;
      // WebTorrent auto-selects every file (downloads the whole torrent). We only ever
      // play one file, so deselect them all up front; the chosen file is selected later.
      // Range reads through the service worker still fetch the pieces they need on demand.
      for (const f of t.files) f.deselect();
      currentTorrent = t;
      resolve(t);
    });
    torrent.on('error', (err) => {
      if (!settled) reject(err instanceof Error ? err : new Error(String(err)));
    });
    torrent.on('warning', (w) => console.warn('[webtorrent]', w?.message || w));
  });
}

/**
 * Prioritise downloading the chosen file and return its service-worker stream URL.
 * The URL serves the file over HTTP with byte-range support, ready to hand to the player.
 */
export function streamUrlFor(file) {
  for (const f of (currentTorrent?.files || [])) f.deselect();
  file.select();
  return file.streamURL;
}
