use std::cell::RefCell;
use std::cmp::min;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use gloo_net::http::Request;
use js_sys::{Reflect, Uint8Array};
use lru::LruCache;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::ReadableStreamDefaultReader;
use matroska_ebml::ebml::{EbmlSource, Size};

/// Cache/fetch granularity. Larger blocks mean far fewer HTTP round-trips when the
/// EBML reader makes many small adjacent reads (vint headers, cluster headers).
const BLOCK: u64 = 16_384;

/// Largest forward gap we'll keep an open stream alive across: rather than abandoning the
/// stream and issuing a fresh request, we read (and cache) the skipped bytes. A jump
/// bigger than this is a seek — we cancel the stream and reopen at the new offset, so we
/// never stream across half the file just because the tail is needed.
const SEQ_GAP: u64 = 512 * 1024;

/// Read-ahead size for the *fallback* range path (used only when the body can't be
/// streamed, e.g. a server that ignores Range or omits a readable body). Bounded so this
/// path never over-fetches far past what was asked for.
const FALLBACK_WINDOW: u64 = 1024 * 1024;

/// How far each streaming request reaches ahead. An open-ended `bytes=START-` lets the
/// browser (or, worse, a WebTorrent service worker) eagerly download the whole rest of the
/// file into memory no matter how slowly we read it — hundreds of MB of wasted bandwidth
/// and RAM. Capping each request to a segment bounds the read-ahead: we pull through one
/// segment, then open the next as playback reaches it. A single read larger than this
/// (a big cluster) extends its segment to cover itself.
const SEGMENT: u64 = 16 * 1024 * 1024;

/// When true, every network request (stream open + range fetch) is logged to the console,
/// so the request count for a playback session can be eyeballed in the browser dev tools.
const LOG_REQUESTS: bool = true;

thread_local! {
    // 16 KB blocks × 4096 entries ≈ 64 MB cap. This single cache backs every read: a live
    // stream flushes blocks into it as they arrive, so the trailing audio pass (which
    // re-reads the same clusters the video pass just streamed) is served entirely from here
    // without ever touching the network.
    pub static READ_CACHE: RefCell<LruCache<String, Box<[u8]>>> = RefCell::new(
      LruCache::new(NonZeroUsize::new(4096).unwrap())
    );

    // One open, forward-only HTTP stream per URL. Held across awaits by `remove`-ing it
    // while in use and reinserting afterwards — sound because the MSE driver feeds tracks
    // sequentially, so there is never more than one in-flight read.
    static LIVE: RefCell<HashMap<String, LiveStream>> = RefCell::new(HashMap::new());
}

/// A single long-lived `bytes=START-` response, consumed incrementally. Its bytes are
/// flushed into `READ_CACHE` block by block as they arrive, so linear playback costs one
/// HTTP request for the whole forward run instead of one request per cluster.
struct LiveStream {
    /// Reader over the response body's `ReadableStream`.
    reader: ReadableStreamDefaultReader,
    /// Absolute offset this stream was opened at — the earliest byte it can serve.
    origin: u64,
    /// Absolute offset of the next byte the network will deliver (the stream head).
    next: u64,
    /// Bytes pulled but not yet forming a complete block; `buf[0]` is at `buf_start`.
    buf: Vec<u8>,
    /// Absolute, block-aligned offset of `buf[0]`.
    buf_start: u64,
    /// Whether the stream reached end-of-body.
    eof: bool,
}

#[derive(Clone)]
pub struct StreamSource {
    pub url: String
}


impl PartialEq for StreamSource {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
    }
}

impl StreamSource {
    pub fn new(url: String) -> StreamSource {
        StreamSource {
            url,
        }
    }

    /// Speculative cold-start prefetch: fetch the first 32 KB and the last 256 KB
    /// **in parallel** (before any parsing) and seed the block cache, so the initial
    /// header/Tracks parse and the trailing Cues read both hit warm cache. The suffix
    /// range (`bytes=-N`) retrieves the tail without first knowing the file size; the
    /// 206 `Content-Range` header tells us where it landed.
    pub async fn prefetch(&self) {
        let head = fetch_with_range(&self.url, "bytes=0-32767".to_string());
        let tail = fetch_with_range(&self.url, "bytes=-262144".to_string());
        let (head, tail) = futures::join!(head, tail);

        // The head always starts at byte 0, so it's safe to seed regardless of
        // whether the response exposed Content-Range.
        if let Some((_, data)) = head {
            seed_cache(&self.url, 0, &data);
        }
        // The tail's absolute offset is only known from Content-Range. Cross-origin
        // servers frequently don't expose that header — in which case we MUST NOT
        // guess (seeding at the wrong offset would clobber other cached blocks).
        // The prefetch is only a warm-up, so skipping it is harmless.
        if let Some((Some(start), data)) = tail {
            seed_cache(&self.url, start, &data);
        }
    }
}

fn log_request(msg: &str) {
    if LOG_REQUESTS {
        web_sys::console::log_1(&format!("[fetch] {}", msg).into());
    }
}

/// Fetch a byte range, returning `(content_range_start, data)` on a 206 response.
/// `content_range_start` is `None` when the `Content-Range` header is absent or
/// unreadable (e.g. not exposed across origins).
async fn fetch_with_range(url: &str, range: String) -> Option<(Option<u64>, Vec<u8>)> {
    log_request(&format!("range {}", range));
    let resp = Request::get(url).header("Range", &range).send().await.ok()?;
    if resp.status() != 206 {
        return None;
    }
    let start = resp
        .headers()
        .get("content-range")
        .and_then(|cr| parse_content_range_start(&cr));
    let data = resp.binary().await.ok()?;
    Some((start, data))
}

/// Extract the start offset from a `Content-Range: bytes START-END/TOTAL` header.
fn parse_content_range_start(header: &str) -> Option<u64> {
    let after_unit = header.trim().strip_prefix("bytes ")?;
    let range = after_unit.split('/').next()?;
    range.split('-').next()?.trim().parse().ok()
}

/// Seed the block LRU with `data` that begins at absolute offset `abs_start`.
/// Only blocks aligned to the cache's `BLOCK`-byte grid are stored; a partial leading
/// region (when `abs_start` is unaligned) is left for the normal read path to fetch.
fn seed_cache(url: &str, abs_start: u64, data: &[u8]) {
    let end = abs_start + data.len() as u64;
    let first_block = abs_start.div_ceil(BLOCK) * BLOCK;
    let mut off = first_block;
    while off < end {
        let rel = (off - abs_start) as usize;
        let take = min(BLOCK as usize, data.len() - rel);
        let block = data[rel..rel + take].to_vec().into_boxed_slice();
        READ_CACHE.with(|cache| {
            cache.borrow_mut().put(format!("{}-{}", url, off), block);
        });
        off += BLOCK;
    }
}

/// Raw byte-range fetch (whole body buffered). Used only for prefetch and the fallback
/// fill path; the hot playback path streams instead (see `fill`).
async fn fetch_range(url: &str, start: Size, end: Size) -> Vec<u8> {
    log_request(&format!("range {}-{}", start, end));
    let resp = Request::get(url)
        .header("Range", format!("bytes={}-{}", start, end).as_str())
        .send()
        .await
        .unwrap();

    if resp.status() != 206 {
        return Vec::new();
    }

    resp.binary().await.unwrap()
}

fn read_cache_direct(url: &str, key: u64) -> Option<Box<[u8]>> {
    READ_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.get(format!("{}-{}", url, key).as_str()).map(|v| v.clone())
    })
}

/// Whether a block is cached, without bumping its LRU recency (used for gap detection).
fn read_cache_contains(url: &str, key: u64) -> bool {
    READ_CACHE.with(|cache| cache.borrow().contains(format!("{}-{}", url, key).as_str()))
}

/// Is any block of `[start, end]` absent from the cache?
fn blocks_missing(url: &str, start: Size, end: Size) -> bool {
    (start / BLOCK..=end / BLOCK).any(|q| !read_cache_contains(url, q * BLOCK))
}

/// Block-aligned offset of the first absent block in `[start, end]`, if any.
fn first_missing_aligned(url: &str, start: Size, end: Size) -> Option<u64> {
    (start / BLOCK..=end / BLOCK)
        .find(|&q| !read_cache_contains(url, q * BLOCK))
        .map(|q| q * BLOCK)
}

// ----------------------------------------------------------------------------
// Live streaming
// ----------------------------------------------------------------------------

fn take_live(url: &str) -> Option<LiveStream> {
    LIVE.with(|m| m.borrow_mut().remove(url))
}

fn put_live(url: &str, stream: LiveStream) {
    LIVE.with(|m| {
        m.borrow_mut().insert(url.to_string(), stream);
    });
}

/// Abort an open stream so its connection is released (browsers cap concurrent
/// connections per origin — a leaked reader would eventually stall new requests).
fn cancel_stream(stream: LiveStream) {
    let _ = stream.reader.cancel();
}

/// Cancel any open streams for *other* URLs. Only one video plays at a time, so when we
/// touch a URL's stream we can release a previous video's connection (browsers cap
/// concurrent connections per origin, and an abandoned reader would otherwise linger).
fn purge_streams_except(url: &str) {
    LIVE.with(|m| {
        let mut m = m.borrow_mut();
        let others: Vec<String> = m.keys().filter(|k| k.as_str() != url).cloned().collect();
        for k in others {
            if let Some(s) = m.remove(&k) {
                let _ = s.reader.cancel();
            }
        }
    });
}

/// Can this stream serve the read whose first still-missing block is at `gap_start`
/// without reopening? Yes as long as it's live, `gap_start` is not before what the stream
/// can cover (`origin`), and it's not a big forward jump past the head. A `gap_start` that
/// sits *between* `origin` and the head (a block evicted from cache that the stream already
/// streamed past) keeps the stream — that read is served by the bounded range fallback
/// while the forward stream stays intact, instead of a backward reopen that would kill it.
fn stream_usable(stream: &LiveStream, gap_start: u64) -> bool {
    !stream.eof && gap_start >= stream.origin && gap_start <= stream.next + SEQ_GAP
}

/// Open a forward stream at `start` (which must be block-aligned), reaching at least to
/// `min_end` and otherwise one `SEGMENT` ahead. The bounded range is what keeps the
/// browser/torrent from eagerly downloading the whole file. Returns `None` if the server
/// didn't answer 206 (a 200 would start at byte 0, corrupting offsets) or didn't expose a
/// readable body — the caller then falls back to range requests.
async fn open_stream(url: &str, start: u64, min_end: u64) -> Option<LiveStream> {
    // Align the segment end up to a block boundary so it never ends mid-block (a short
    // block mid-file would corrupt a later read of that block); only the real EOF block,
    // where the server returns fewer bytes than requested, is allowed to be short.
    let reach = (start + SEGMENT - 1).max(min_end);
    let seg_end = (reach / BLOCK + 1) * BLOCK - 1;
    log_request(&format!("stream open {}-{}", start, seg_end));
    let resp = Request::get(url)
        .header("Range", format!("bytes={}-{}", start, seg_end).as_str())
        .send()
        .await
        .ok()?;
    if resp.status() != 206 {
        return None;
    }
    let reader: ReadableStreamDefaultReader = resp.body()?.get_reader().unchecked_into();
    Some(LiveStream {
        reader,
        origin: start,
        next: start,
        buf: Vec::new(),
        buf_start: start,
        eof: false,
    })
}

/// Move every complete block out of `buf` into the cache. Copies the block slices, then
/// drains the consumed prefix *once* — draining a block at a time would re-shift the rest
/// of `buf` on every block (quadratic when a network chunk carries many blocks).
fn flush_full_blocks(url: &str, stream: &mut LiveStream) {
    let block = BLOCK as usize;
    let complete = (stream.buf.len() / block) * block;
    if complete == 0 {
        return;
    }
    let mut off = 0;
    while off < complete {
        let data = stream.buf[off..off + block].to_vec().into_boxed_slice();
        READ_CACHE.with(|c| {
            c.borrow_mut().put(format!("{}-{}", url, stream.buf_start), data)
        });
        stream.buf_start += BLOCK;
        off += block;
    }
    stream.buf.drain(..complete);
}

/// At EOF, flush the trailing partial block (the file's last, short block). Reads landing
/// here are clamped during assembly, so a short final block is safe.
fn flush_remainder(url: &str, stream: &mut LiveStream) {
    if !stream.buf.is_empty() {
        let len = stream.buf.len() as u64;
        let block = std::mem::take(&mut stream.buf);
        READ_CACHE.with(|c| {
            c.borrow_mut()
                .put(format!("{}-{}", url, stream.buf_start), block.into_boxed_slice())
        });
        stream.buf_start += len;
    }
}

/// Pull one chunk from the stream into the cache. `Some(true)` = data added,
/// `Some(false)` = end-of-body reached, `None` = network/read error.
async fn pull(url: &str, stream: &mut LiveStream) -> Option<bool> {
    let result = JsFuture::from(stream.reader.read()).await.ok()?;
    let done = Reflect::get(&result, &JsValue::from_str("done"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if done {
        flush_remainder(url, stream);
        stream.eof = true;
        return Some(false);
    }
    let value = Reflect::get(&result, &JsValue::from_str("value")).ok()?;
    let chunk = value.unchecked_into::<Uint8Array>().to_vec();
    if !chunk.is_empty() {
        stream.next += chunk.len() as u64;
        stream.buf.extend_from_slice(&chunk);
        flush_full_blocks(url, stream);
    }
    Some(true)
}

/// Pull until the stream head passes `target_excl` (or EOF). Returns `false` only on a
/// network error, so the caller can reopen; EOF is a normal stop and returns `true`.
async fn pull_until(url: &str, stream: &mut LiveStream, target_excl: u64) -> bool {
    while !stream.eof && stream.next < target_excl {
        match pull(url, stream).await {
            Some(true) => {}
            Some(false) => break,
            None => return false,
        }
    }
    true
}

/// Ensure every block of `[start, end]` is in the cache, preferring the live stream and
/// falling back to a bounded range request.
async fn fill(url: &str, start: Size, end: Size) {
    let target_excl = (end / BLOCK + 1) * BLOCK;

    // Drive the decision off the first block we still need, not the read's start: a read
    // often begins inside data the stream already delivered (the cluster's header peek
    // streamed its first block), and reopening there would needlessly kill the stream.
    let Some(gap_start) = first_missing_aligned(url, start, end) else {
        return;
    };

    purge_streams_except(url);

    let mut stream = take_live(url);
    // Reopen unless the current stream can continue to this gap (else it's a seek, an
    // idle-closed stream, or there is none yet).
    let reopen = stream.as_ref().map_or(true, |s| !stream_usable(s, gap_start));
    if reopen {
        if let Some(old) = stream.take() {
            cancel_stream(old);
        }
        stream = open_stream(url, gap_start, end).await;
    }

    if let Some(mut s) = stream {
        if pull_until(url, &mut s, target_excl).await {
            put_live(url, s);
        } else {
            // The stream died mid-flight — typically a proxy closing an idle connection
            // after the player was paused. Reopen once at the first still-missing block and
            // resume as a single forward stream rather than degrading to per-read requests.
            cancel_stream(s);
            let resume = first_missing_aligned(url, start, end).unwrap_or(gap_start);
            if let Some(mut resumed) = open_stream(url, resume, end).await {
                let _ = pull_until(url, &mut resumed, target_excl).await;
                put_live(url, resumed);
            }
        }
    }

    // Streaming unavailable or still short (non-206 server, repeated failure): guarantee
    // correctness with a bounded range fetch of whatever is still missing.
    if let Some(lo) = first_missing_aligned(url, start, end) {
        let needed_end = (end / BLOCK + 1) * BLOCK - 1;
        let mut fetch_end = needed_end.max(lo + FALLBACK_WINDOW - 1);
        fetch_end = (fetch_end / BLOCK + 1) * BLOCK - 1;
        let data = fetch_range(url, lo, fetch_end).await;
        if !data.is_empty() {
            seed_cache(url, lo, &data);
        }
    }
}

/// Copy `[start, end]` out of the cache, clamping at EOF / short trailing blocks so a read
/// past end-of-file returns what exists instead of panicking.
fn assemble(url: &str, start: Size, end: Size) -> Vec<u8> {
    let mut result: Vec<u8> = Vec::with_capacity((end - start + 1) as usize);
    let mut off = start;
    while off <= end {
        let block_key = (off / BLOCK) * BLOCK;
        let Some(block) = read_cache_direct(url, block_key) else {
            break;
        };
        let inner = (off - block_key) as usize;
        if inner >= block.len() {
            break;
        }
        let take = min((end - off + 1) as usize, block.len() - inner);
        result.extend_from_slice(&block[inner..inner + take]);
        off += take as u64;
    }
    result
}

/// The single read path for all sizes: serve from cache, filling via the live stream on a
/// miss. Cache-first is load-bearing — it lets the audio pass and prefetched head/tail be
/// served without ever touching (and thus without reopening) the forward stream.
async fn cached_read(url: &str, start: Size, end: Size) -> Vec<u8> {
    if blocks_missing(url, start, end) {
        fill(url, start, end).await;
    }
    assemble(url, start, end)
}


impl EbmlSource for StreamSource {

    async fn read_range(&self, start: Size, end: Size) -> Vec<u8> {
        cached_read(&self.url, start, end).await
    }

    async fn read_exact(&self, start: Size, length: usize) -> Vec<u8> {
        let end = start + length as u64 - 1;
        cached_read(&self.url, start, end).await
    }

}
