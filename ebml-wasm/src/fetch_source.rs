use std::cell::{RefCell, RefMut};
use std::cmp::min;
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};
use cache_rs::config::LfuCacheConfig;
use cache_rs::{LfuCache, SIZE_UNIT};
use gloo_net::http::Request;
use lru::LruCache;
use crate::ebml::{EbmlSource, Size};

/// Cache/fetch granularity. Larger blocks mean far fewer HTTP round-trips when the
/// EBML reader makes many small adjacent reads (vint headers, cluster headers).
const BLOCK: u64 = 16_384;

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

/// Seed the 1 KB block LRU with `data` that begins at absolute offset `abs_start`.
/// Only blocks aligned to the cache's 1024-byte grid are stored; a partial leading
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

async fn read_cache_entry(url: &str, start: Size, end: Size) -> Vec<u8> {
    let quotient_start = start / BLOCK;
    let remainder_start = (start % BLOCK) as usize;

    let quotient_end = end / BLOCK;
    let remainder_end = (end % BLOCK) as usize;

    let mut result: Vec<u8> = Vec::new();

    let mut fetch_start: Option<Size> = None;
    let mut fetch_end: Option<Size> = None;

    for quotient in quotient_start..=quotient_end {
        let key = quotient * BLOCK;
        let value = read_cache_direct(url, key);

        if let Some(value) = value {
            if let Some(some_fetch_start) = fetch_start && let Some(some_fetch_end) = fetch_end {
                let mut data = read_range(url, some_fetch_start, some_fetch_end).await;

                result.append(&mut data);

                fetch_start = None;
                fetch_end = None;
            }

            result.extend(value);
        } else {
            if fetch_start.is_none() {
                fetch_start = Some(quotient * BLOCK);
            }
            fetch_end = Some((quotient + 1) * BLOCK - 1);
        }
    }

    if let Some(some_fetch_start) = fetch_start && let Some(some_fetch_end) = fetch_end {
        let mut data = read_range(url, some_fetch_start, some_fetch_end).await;

        result.append(&mut data);

        fetch_start = None;
        fetch_end = None;
    }

    for quotient in quotient_start..=quotient_end {
        let key = quotient * BLOCK;

        let slice_start = quotient * BLOCK;
        let slice_end = (quotient + 1) * BLOCK - 1;

        let start_range = (slice_start - (quotient_start * BLOCK)) as usize;
        let end_range = ((slice_end - (quotient_start * BLOCK))) as usize;

        let end_range = min(end_range, result.len() - 1);

        if result.len() == 0 {
            break;
        }

        let slice = result[start_range..=end_range].to_vec().into_boxed_slice();

        READ_CACHE.with(|cache| {
            cache.borrow_mut().put(
                format!("{}-{}", url, key),
                slice,
            )
        });
    }

    let end_range = ((quotient_end - quotient_start) * BLOCK) as usize + remainder_end;
    let end_range = min(end_range, result.len() - 1);

    // web_sys::console::log_1(&format!("{} - {}, Result: {:?}", remainder_start, end_range, result[remainder_start..=end_range].to_vec()).into());

    if result.len() == 0 {
        return result;
    }

    // web_sys::console::log_1(&format!("READ: {} - {} = {:?}", start, end, result[remainder_start..=end_range].to_vec()).into());

    result[remainder_start..=end_range].to_vec()
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