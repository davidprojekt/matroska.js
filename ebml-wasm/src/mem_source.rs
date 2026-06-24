//! An in-memory [`EbmlSource`] over a byte buffer with a base offset. Used to read a
//! whole element (Cues/Tracks) or a whole Cluster in a single network request and
//! then parse it field-by-field for free, instead of issuing a request per field.

use crate::ebml::{EbmlSource, Size};
use std::cmp::min;
use std::rc::Rc;

#[derive(Clone)]
pub struct MemSource {
    data: Rc<[u8]>,
    /// Absolute file offset corresponding to `data[0]`.
    base: u64,
}

impl MemSource {
    pub fn new(data: Vec<u8>, base: u64) -> Self {
        Self {
            data: data.into(),
            base,
        }
    }
}

impl PartialEq for MemSource {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base && Rc::ptr_eq(&self.data, &other.data)
    }
}

impl EbmlSource for MemSource {
    async fn read_range(&self, start: Size, end: Size) -> Vec<u8> {
        if start < self.base {
            return Vec::new();
        }
        let s = (start - self.base) as usize;
        if s >= self.data.len() {
            return Vec::new();
        }
        let e = min((end - self.base) as usize, self.data.len() - 1);
        if e < s {
            return Vec::new();
        }
        self.data[s..=e].to_vec()
    }

    async fn read_exact(&self, start: Size, length: usize) -> Vec<u8> {
        if length == 0 {
            return Vec::new();
        }
        self.read_range(start, start + length as u64 - 1).await
    }
}
