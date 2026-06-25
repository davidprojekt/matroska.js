use std::cell::RefCell;
use std::cmp::min;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use gloo_net::http::Request;
use lru::LruCache;
use crate::ebml::{EbmlSource, Size};

/// Cache/fetch granularity. Larger blocks mean far fewer HTTP round-trips when the
/// EBML reader makes many small adjacent reads (vint headers, cluster headers).
const BLOCK: u64 = 16_384;

/// Read-ahead tuning. On *sequential* access we fetch a window that grows with each
/// consecutive hit, so a long forward scan (the header parse, a cluster walk) costs a
/// handful of requests instead of one per block. The skipped "gap" bytes between two
/// nearby reads are pulled once inside the window and cached, so the second read is a
/// cache hit rather than a fresh request — i.e. we "stream over" small gaps.
const WINDOW_BASE: u64 = 64 * 1024;
/// Upper bound on a single read-ahead fetch. This is what stops us from streaming over
/// half the file when only a small region (e.g. the tail Cues) is actually needed.
const WINDOW_CAP: u64 = 4 * 1024 * 1024;
/// Largest forward gap from the last fetched byte still treated as "sequential". A jump
/// bigger than this is a seek: we drop the read-ahead window and make a fresh, tight
/// request rather than wastefully streaming across the gap.
const SEQ_GAP: u64 = 512 * 1024;

/// Per-URL read-ahead state driving the windowing heuristic.
#[derive(Clone, Copy)]
struct ReadAhead {
    /// Absolute offset just past the last byte we fetched (the "stream head").
    last_end: u64,
    /// Window size to use for the next sequential fetch.
    window: u64,
}

thread_local! {
    // 16 KB blocks × 4096 entries ≈ 64 MB cap.
    pub static READ_CACHE: RefCell<LruCache<String, Box<[u8]>>> = RefCell::new(
      LruCache::new(NonZeroUsize::new(4096).unwrap())
    );

    // Large (whole-cluster) reads keyed by exact range. Video and audio passes request
    // the same clusters, so the second pass reuses the first's fetch instead of
    // re-downloading. Small: only the in-flight buffer window of clusters.
    static CLUSTER_CACHE: RefCell<LruCache<String, Box<[u8]>>> = RefCell::new(
      LruCache::new(NonZeroUsize::new(32).unwrap())
    );

    // Tracks the forward read position per URL so adjacent reads can be coalesced into
    // a single growing fetch instead of one request per block.
    static READ_AHEAD: RefCell<HashMap<String, ReadAhead>> = RefCell::new(HashMap::new());
}

fn read_ahead_get(url: &str) -> ReadAhead {
    READ_AHEAD.with(|m| {
        m.borrow()
            .get(url)
            .copied()
            .unwrap_or(ReadAhead { last_end: 0, window: WINDOW_BASE })
    })
}

fn read_ahead_set(url: &str, state: ReadAhead) {
    READ_AHEAD.with(|m| {
        m.borrow_mut().insert(url.to_string(), state);
    });
}

/// Advance the stream head after a whole-cluster read, so a small read that immediately
/// follows it (the next cluster's header) still counts as sequential. The window is reset
/// to a single block: a cluster read ends any small-read streak, and the header peek that
/// follows sits right in front of *another* large read — reading ahead there would just
/// re-download bytes the upcoming cluster fetch already covers.
fn note_fetched(url: &str, end_exclusive: u64) {
    let mut state = read_ahead_get(url);
    state.window = BLOCK;
    if end_exclusive > state.last_end {
        state.last_end = end_exclusive;
    }
    read_ahead_set(url, state);
}

#[derive(Clone)]
pub struct FetchSource {
    pub url: String
}


impl PartialEq for FetchSource {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
    }
}

impl FetchSource {
    pub fn new(url: String) -> FetchSource {
        FetchSource {
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

/// Fetch a byte range, returning `(content_range_start, data)` on a 206 response.
/// `content_range_start` is `None` when the `Content-Range` header is absent or
/// unreadable (e.g. not exposed across origins).
async fn fetch_with_range(url: &str, range: String) -> Option<(Option<u64>, Vec<u8>)> {
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

/// Whole-cluster read with an exact-range cache, so the audio pass reuses the
/// cluster bytes the video pass already fetched (and vice versa).
async fn read_large_cached(url: &str, start: Size, end: Size) -> Vec<u8> {
    let key = format!("{}-{}-{}", url, start, end);
    if let Some(hit) = CLUSTER_CACHE.with(|c| c.borrow_mut().get(&key).map(|v| v.to_vec())) {
        return hit;
    }
    let data = read_range(url, start, end).await;
    if !data.is_empty() {
        CLUSTER_CACHE.with(|c| c.borrow_mut().put(key, data.clone().into_boxed_slice()));
        // Keep the stream head in sync so a small read right after this cluster (the next
        // cluster's header) is still recognised as sequential.
        note_fetched(url, start + data.len() as u64);
    }
    data
}

async fn read_range(url: &str, start: Size, end: Size) -> Vec<u8> {
    let resp = Request::get(url)
        .header("Range", format!("bytes={}-{}", start, end).as_str())
        .send()
        .await
        .unwrap();

    // assert_eq!(resp.status(), 206);

    if resp.status() != 206 {
        return Vec::new();
    }

    let data = resp.binary().await.unwrap();

    data
}

fn read_cache_direct(url: &str, key: u64) -> Option<Box<[u8]>> {
    let value = READ_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let option = cache.get(format!("{}-{}", url, key).as_str());
        option.map(|v| v.clone())
    });

    value
}

/// Whether a block is cached, without bumping its LRU recency (used for gap detection).
fn read_cache_contains(url: &str, key: u64) -> bool {
    READ_CACHE.with(|cache| cache.borrow().contains(format!("{}-{}", url, key).as_str()))
}

/// Read a small range through the block cache. On a miss, fetch a *read-ahead window*
/// rather than just the missing block(s): sequential scans then download in a few large,
/// growing chunks instead of one request per 16 KB. A seek (backward, or a forward jump
/// larger than `SEQ_GAP`) resets the window so we never stream across a big gap.
async fn read_cache_entry(url: &str, start: Size, end: Size) -> Vec<u8> {
    let quotient_start = start / BLOCK;
    let quotient_end = end / BLOCK;

    // Find the contiguous span of missing blocks within the request (at most two blocks,
    // since callers only take this path for reads up to BLOCK bytes).
    let mut miss_lo: Option<u64> = None;
    let mut miss_hi: Option<u64> = None;
    for quotient in quotient_start..=quotient_end {
        if !read_cache_contains(url, quotient * BLOCK) {
            if miss_lo.is_none() {
                miss_lo = Some(quotient);
            }
            miss_hi = Some(quotient);
        }
    }

    if let (Some(lo), Some(hi)) = (miss_lo, miss_hi) {
        let needed_start = lo * BLOCK;
        let needed_end = (hi + 1) * BLOCK - 1;

        // Sequential iff the missing region continues roughly where the last fetch ended:
        // within `SEQ_GAP` ahead, tolerating up to one block of overlap behind (a read at a
        // cluster boundary lands in the block that *starts* just before `last_end`).
        // Anything else is a seek and falls back to a tight, base-sized window.
        let state = read_ahead_get(url);
        let sequential = needed_start + BLOCK >= state.last_end
            && needed_start <= state.last_end + SEQ_GAP;
        let window = if sequential { state.window } else { WINDOW_BASE };

        // Read ahead up to `window`, but always at least the bytes actually needed. Align
        // the tail to the block grid so every cached block stays full-sized.
        let mut fetch_end = needed_end.max(needed_start + window - 1);
        fetch_end = (fetch_end / BLOCK + 1) * BLOCK - 1;

        let data = read_range(url, needed_start, fetch_end).await;
        if !data.is_empty() {
            // `needed_start` is block-aligned, so this seeds whole blocks (the trailing one
            // may be short at EOF, which the assembly step below clamps for).
            seed_cache(url, needed_start, &data);
        }

        let covered_end = needed_start + data.len() as u64;
        read_ahead_set(
            url,
            ReadAhead {
                last_end: covered_end.max(state.last_end),
                window: min(window.saturating_mul(2), WINDOW_CAP),
            },
        );
    }

    // Assemble the requested range from the (now populated) cache, clamping at EOF / short
    // trailing blocks so a read past the end of file returns what exists instead of panicking.
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


impl EbmlSource for FetchSource {

    async fn read_range(&self, start: Size, end: Size) -> Vec<u8> {
        let size = end - start;
        if size > BLOCK {
            read_large_cached(&self.url, start, end).await
        } else {
            read_cache_entry(&self.url, start, end).await
        }
    }

    async fn read_exact(&self, start: Size, length: usize) -> Vec<u8> {
        let end = start + length as u64 - 1;
        if length as u64 > BLOCK {
            read_large_cached(&self.url, start, end).await
        } else {
            read_cache_entry(&self.url, start, end).await
        }
    }

}