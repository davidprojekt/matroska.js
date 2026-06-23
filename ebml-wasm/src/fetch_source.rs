use std::cell::{RefCell, RefMut};
use std::cmp::min;
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};
use cache_rs::config::LfuCacheConfig;
use cache_rs::{LfuCache, SIZE_UNIT};
use gloo_net::http::Request;
use lru::LruCache;
use crate::ebml::{EbmlSource, Size};

thread_local! {
    pub static READ_CACHE: RefCell<LruCache<String, Box<[u8]>>> = RefCell::new(
      LruCache::new(NonZeroUsize::new(10_000).unwrap())
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
    let quotient_start = start / 1024;
    let remainder_start = (start % 1024) as usize;

    let quotient_end = end / 1024;
    let remainder_end = (end % 1024) as usize;

    let mut result: Vec<u8> = Vec::new();

    let mut fetch_start: Option<Size> = None;
    let mut fetch_end: Option<Size> = None;

    for quotient in quotient_start..=quotient_end {
        let key = quotient * 1024;
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
                fetch_start = Some(quotient * 1024);
            }
            fetch_end = Some((quotient + 1) * 1024 - 1);
        }
    }

    if let Some(some_fetch_start) = fetch_start && let Some(some_fetch_end) = fetch_end {
        let mut data = read_range(url, some_fetch_start, some_fetch_end).await;

        result.append(&mut data);

        fetch_start = None;
        fetch_end = None;
    }

    for quotient in quotient_start..=quotient_end {
        let key = quotient * 1024;

        let slice_start = quotient * 1024;
        let slice_end = (quotient + 1) * 1024 - 1;

        let start_range = (slice_start - (quotient_start * 1024)) as usize;
        let end_range = ((slice_end - (quotient_start * 1024))) as usize;

        let end_range = min(end_range, result.len() - 1);

        if result.len() == 0 {
            break;
        }


        // web_sys::console::log_1(&format!("Start: {}, End: {}", start_range, end_range).into());

        let slice = result[start_range..=end_range].to_vec().into_boxed_slice();

        // web_sys::console::log_1(&format!("Slice: {:?}", slice).into());

        READ_CACHE.with(|cache| {
            cache.borrow_mut().put(
                format!("{}-{}", url, key),
                slice,
            )
        });
    }

    let end_range = ((quotient_end - quotient_start) * 1024) as usize + remainder_end;
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
        if size > 10_000 {
            read_range(&self.url, start, end).await
        } else {
            read_cache_entry(&self.url, start, end).await
        }
    }

    async fn read_exact(&self, start: Size, length: usize) -> Vec<u8> {
        if length > 10_000 {
            read_range(&self.url, start, start + length as u64 - 1).await
        } else {
            read_cache_entry(&self.url, start, start + length as u64 - 1).await
        }
    }

}