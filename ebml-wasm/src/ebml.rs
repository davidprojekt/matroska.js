use std::collections::HashMap;
use std::fmt::Debug;
use crate::matroska_data::EbmlType;

pub type Size = u64;

pub trait EbmlSource {
    async fn read_range(&self, start: Size, end: Size) -> Vec<u8>;
    async fn read_exact(&self, start: Size, length: usize) -> Vec<u8>;
}

#[derive(Clone, Debug)]
pub struct Ebml<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> where S: Clone {
    pub source: S,
    pub id_map: HashMap<u64, EbmlType>,
}


impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> PartialEq for Ebml<S> {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

// https://www.matroska.org/technical/elements.html
#[derive(Debug, Clone, PartialEq)]
pub struct EbmlElement<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> {
    pub id: u64,
    pub offset: Size,
    pub size: Size,
    pub payload: EbmlPayload<S>,
}

#[derive(Debug, Clone)]
pub enum EbmlPayload<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> {
    Master(EbmlIterator<S>),
    UnsignedInt(u64),
    SignedInt(i64),
    Float(f64),
    Date(i64),
    String(String),
    Binary((u64, u64)),
    Void,

    Unknown((u64, u64)),
    Invalid(Option<(u64, u64)>),
}

impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> PartialEq for EbmlPayload<S> {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}

impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> EbmlElement<S> {
    pub fn empty() -> Self {
        EbmlElement {
            id: 0,
            offset: 0,
            size: 0,
            payload: EbmlPayload::Void,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeekableEbmlIterator<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> {
    iter: EbmlIterator<S>,
    /// Caches the peeked element.
    /// Outer Option: Has a peek been performed?
    /// Inner Option: The actual result of iter.next().await
    peeked: Option<Option<EbmlElement<S>>>,
}

impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> PeekableEbmlIterator<S> {
    pub fn new(iter: EbmlIterator<S>) -> Self {
        Self {
            iter,
            peeked: None,
        }
    }

    /// Returns a reference to the next element without consuming it.
    pub async fn peek(&mut self) -> Option<&EbmlElement<S>> {
        if self.peeked.is_none() {
            // We haven't peeked yet, so await the next element and cache it.
            self.peeked = Some(self.iter.next().await);
        }

        // Unwrap the outer Option (safe because we just populated it),
        // then return a reference to the inner Option's contents.
        self.peeked.as_ref().unwrap().as_ref()
    }

    /// Consumes and returns the next element, utilizing the cache if it exists.
    pub async fn next(&mut self) -> Option<EbmlElement<S>> {
        // If we have a cached value, take it (leaving None in self.peeked)
        if let Some(peeked_result) = self.peeked.take() {
            peeked_result
        } else {
            // Otherwise, fetch directly from the underlying iterator
            self.iter.next().await
        }
    }

    /// Gives back the inner iterator, destroying the peekable wrapper
    pub fn into_inner(self) -> EbmlIterator<S> {
        self.iter
    }
}

#[derive(Debug, Clone)]
pub struct EbmlIterator<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> {
    pub current: u64,
    pub end: Option<u64>,
    pub ebml: Ebml<S>,
}

impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> EbmlIterator<S> {
    pub fn new_endless(offset: u64, ebml: Ebml<S>) -> Self {
        Self {
            current: offset,
            end: None,
            ebml: ebml,
        }
    }

    pub fn new(offset: u64, end: u64, ebml: Ebml<S>) -> Self {
        Self {
            current: offset,
            end: Some(end),
            ebml: ebml,
        }
    }

    pub async fn read_range(&self, start: Size, end: Size) -> Box<[u8]> {
        self.ebml.source.read_range(start, end).await.into_boxed_slice()
    }
}

impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> EbmlIterator<S> {

    pub fn peekable(self) -> PeekableEbmlIterator<S> {
        PeekableEbmlIterator::new(self)
    }

    pub async fn next(&mut self) -> Option<EbmlElement<S>> {
        let offset = self.current;
        let start = self.current;

        if let Some(end) = self.end
            && offset >= end {
                return None;
        }

        let (length, id) = self.ebml.read_variable_size_id(self.current).await;
        self.current += length;

        let (length, size) = self.ebml.read_variable_size_data_size(self.current).await;
        self.current += length;

        if let Some(ebml_type) = self.ebml.id_map.get(&id) {
           // web_sys::console::log_1(&format!(" > {:?}", ebml_type).into());

            let payload: EbmlPayload<S> = match (ebml_type, size) {
                // A Master Element may legitimately have an unknown size
                // (RFC 8794 §6.2): it then extends until an element that is not
                // a valid child, or the end of its parent. We expose it as an
                // endless iterator (`end: None`), which downstream consumers
                // already cap at the parent's end via `end.unwrap_or(...)`.
                //
                // We cannot determine the exact end without element-hierarchy
                // knowledge, so we assume it runs to the end of this parent.
                // This covers the common single trailing master (Segment, or a
                // final Cluster); multiple unknown-size sibling masters (live
                // streaming) are not resolved here.
                (EbmlType::Master, None) => {
                    let elements: EbmlIterator<S> = EbmlIterator::new_endless(self.current, self.ebml.clone());

                    self.current = self.end.unwrap_or(u64::MAX);

                    EbmlPayload::Master(elements)
                }
                (EbmlType::Master, Some(size)) => {
                    let offset_at_size = self.current;

                    let elements: EbmlIterator<S> = EbmlIterator::new(self.current, offset_at_size + size, self.ebml.clone());

                    self.current = offset_at_size + size;

                    EbmlPayload::Master(elements)
                }

                // An unknown-size Void is not meaningful: instead of skipping a
                // bogus (maximal) amount and losing every following element, we
                // resume parsing right after the header.
                (EbmlType::Void, None) => EbmlPayload::Void,
                (EbmlType::Void, Some(size)) => {
                    self.current += size;

                    EbmlPayload::Void
                },

                (EbmlType::SignedInteger, Some(size)) => {
                    if size > 8 {
                        self.current += size;
                        return None;
                    }

                    let bytes = self.ebml.source.read_range(self.current, self.current + size - 1).await;
                    self.current += size;

                    let value = Ebml::<S>::bytes_to_int(bytes);

                    EbmlPayload::SignedInt(value)
                }
                (EbmlType::UnsignedInteger, Some(size)) => {
                    if size > 8 {
                        self.current += size;
                        return None;
                    }

                    let bytes = self.ebml.source.read_range(self.current, self.current + size - 1).await;
                    self.current += size;

                    let value = Ebml::<S>::bytes_to_uint(bytes);

                    EbmlPayload::UnsignedInt(value)
                }
                (EbmlType::Float, Some(size)) => {
                    let bytes = self.ebml.source.read_range(self.current, self.current + size - 1).await;
                    self.current += size;

                    let value = Ebml::<S>::bytes_to_float(bytes);

                    EbmlPayload::Float(value)
                },
                (EbmlType::String, Some(size)) | (EbmlType::UTF8, Some(size)) => {
                    let bytes = self.ebml.source.read_range(self.current, self.current + size - 1).await;
                    self.current += size;

                    let value = Ebml::<S>::bytes_to_string(bytes);

                    EbmlPayload::String(value)
                },
                (EbmlType::Date, Some(size)) => {
                    let bytes = self.ebml.source.read_range(self.current, self.current + size - 1).await;
                    self.current += size;

                    let value = Ebml::<S>::bytes_to_int(bytes);

                    EbmlPayload::Date(value)
                },
                (EbmlType::Binary, Some(size)) => {
                    let offset_at_size = self.current;
                    self.current += size;

                    EbmlPayload::Binary((offset_at_size, offset_at_size + size - 1))
                },
                (EbmlType::Unsupported, Some(size)) => {
                    self.current += size;

                    EbmlPayload::Invalid(None) // unimplemented
                },

                // Any non-Master element with an unknown size is invalid per the
                // spec. Don't skip ahead; resume right after the header so the
                // remaining siblings stay parseable.
                (_, None) => EbmlPayload::Invalid(None),
            };

            let element = EbmlElement {
                id,
                size: self.current - start,
                offset: start,
                payload
            };

            return Some(element);
        }

        None
    }
}

impl<S: EbmlSource + std::cmp::PartialEq + std::clone::Clone> Ebml<S> {
    pub fn new(source: S, id_map: HashMap<u64, EbmlType>) -> Self {
        Ebml {
            source,
            id_map,
        }
    }

    pub fn bytes_to_uint(raw_bytes: Vec<u8>) -> u64 {
        let mut bytes: [u8; 8] = [0; 8];
        let offset = 8 - raw_bytes.len();
        bytes[offset..].copy_from_slice(raw_bytes.as_slice());
        let integer_result = u64::from_be_bytes(bytes);

        integer_result
    }

    pub fn bytes_to_int(raw_bytes: Vec<u8>) -> i64 {
        let mut bytes: [u8; 8] = [0; 8];
        let offset = 8 - raw_bytes.len();
        bytes[offset..].copy_from_slice(raw_bytes.as_slice());
        let integer_result = i64::from_be_bytes(bytes);

        integer_result
    }

    pub fn bytes_to_float(raw_bytes: Vec<u8>) -> f64 {
        // adding left padding zeros to the bytes of a big endian float changes the value
        match raw_bytes.len() {
            0 => 0.0,
            4 => {
                let array: [u8; 4] = raw_bytes.try_into().unwrap();
                f32::from_be_bytes(array) as f64
            }
            8 => {
                let array: [u8; 8] = raw_bytes.try_into().unwrap();
                f64::from_be_bytes(array)
            }
            _ => {
                // EBML specification only allows 0, 4, or 8 byte floats
                panic!("Invalid EBML float length: {} bytes", raw_bytes.len());
            }
        }
    }

    pub fn bytes_to_string(raw_bytes: Vec<u8>) -> String {
        String::from_utf8_lossy(&raw_bytes).into_owned()
    }

    pub async fn read_variable_size_octets(&self, start: Size) -> (Size, Vec<u8>) {
        let mut byte_data = self.source.read_range(start, start).await;
        if byte_data.len() == 0 {
            return (0, vec![])
        }
        let first_byte = byte_data[0];

        let leading_zeros: u64 = first_byte.leading_zeros() as u64;
        let octet_length = leading_zeros + 1;

        if leading_zeros != 0 {
            let mut additional_data = self.source.read_range(start + 1, start + leading_zeros as Size).await;

            byte_data.append(&mut additional_data);
        }

        (octet_length, byte_data)
    }

    pub async fn read_variable_size_id(&self, start: Size) -> (Size, u64) {
        let (octet_length, byte_data) = self.read_variable_size_octets(start).await;

        let mut bytes: [u8; 8] = [0; 8];
        let offset = 8 - byte_data.len();
        bytes[offset..].copy_from_slice(byte_data.as_slice());
        let integer_result = u64::from_be_bytes(bytes);

        (octet_length, integer_result)
    }

    pub async fn read_variable_size_octects_masked(&self, start: Size) -> (Size, Vec<u8>) {
        let (octet_length, mut raw_bytes) = self.read_variable_size_octets(start).await;

        if raw_bytes.len() == 0 {
            return (0, Vec::new())
        }

        // Clear the VINT_WIDTH + VINT_MARKER bits in the leading octet so only
        // the VINT_DATA bits remain. For an 8-octet VINT the entire first octet
        // is the descriptor (0x01), so the whole byte is masked out.
        let mask: u8 = if octet_length >= 8 { 0x00 } else { 0xFF >> octet_length };
        raw_bytes[0] &= mask;

        (octet_length, raw_bytes)
    }

    pub async fn read_variable_size_int(&self, start: Size) -> (Size, i64) {
        let (octet_length, raw_bytes) = self.read_variable_size_octects_masked(start).await;

        let mut bytes: [u8; 8] = [0; 8];
        let offset = 8 - raw_bytes.len();
        bytes[offset..].copy_from_slice(raw_bytes.as_slice());
        let integer_result = i64::from_be_bytes(bytes);

        (octet_length, integer_result)
    }

    pub async fn read_variable_size_uint(&self, start: Size) -> (Size, u64) {
        let (octet_length, raw_bytes) = self.read_variable_size_octects_masked(start).await;

        let mut bytes: [u8; 8] = [0; 8];
        let offset = 8 - raw_bytes.len();
        bytes[offset..].copy_from_slice(raw_bytes.as_slice());
        let integer_result = u64::from_be_bytes(bytes);

        (octet_length, integer_result)
    }

    /// Reads an EBML Element Data Size (a VINT) at `start`.
    ///
    /// Returns `(octet_length, size)`. Per RFC 8794 §6.2, a VINT whose
    /// VINT_DATA bits are *all* set to 1 is the reserved "unknown size"
    /// marker; in that case `size` is `None` instead of the (otherwise
    /// maximal) integer value those bits would represent.
    pub async fn read_variable_size_data_size(&self, start: Size) -> (Size, Option<u64>) {
        let (octet_length, raw_bytes) = self.read_variable_size_octects_masked(start).await;

        if octet_length == 0 {
            return (0, Some(0));
        }

        let mut bytes: [u8; 8] = [0; 8];
        let offset = 8 - raw_bytes.len();
        bytes[offset..].copy_from_slice(raw_bytes.as_slice());
        let value = u64::from_be_bytes(bytes);

        // The number of usable VINT_DATA bits is 7 per octet.
        let data_bits = 7 * octet_length;
        let all_ones = if data_bits >= 64 { u64::MAX } else { (1u64 << data_bits) - 1 };

        let size = if value == all_ones { None } else { Some(value) };

        (octet_length, size)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matroska_data::{ID_TIMESTAMP, ID_VOID};
    use crate::mem_source::MemSource;

    fn ebml(data: Vec<u8>) -> Ebml<MemSource> {
        let mut id_map = HashMap::new();
        id_map.insert(ID_VOID, EbmlType::Void);
        id_map.insert(ID_TIMESTAMP, EbmlType::UnsignedInteger);
        Ebml::new(MemSource::new(data, 0), id_map)
    }

    /// Regression test: a Void element with an *unknown* data size (0xFF) must
    /// not swallow the elements that follow it.
    #[test]
    fn unknown_size_void_does_not_eat_following_elements() {
        // 0xEC 0xFF                -> Void, unknown size
        // 0xE7 0x81 0x2A           -> Timestamp (UnsignedInteger) = 42
        let data = vec![0xEC, 0xFF, 0xE7, 0x81, 0x2A];
        let mut it = EbmlIterator::new_endless(0, ebml(data));

        let void = pollster::block_on(it.next()).expect("void element");
        assert_eq!(void.id, ID_VOID);
        assert!(matches!(void.payload, EbmlPayload::Void));
        // The Void only consumed its 2-byte header, nothing more.
        assert_eq!(void.size, 2);

        let ts = pollster::block_on(it.next()).expect("timestamp element");
        assert_eq!(ts.id, ID_TIMESTAMP);
        assert!(matches!(ts.payload, EbmlPayload::UnsignedInt(42)));
    }

    /// A Void with a normal, known size still skips exactly that many bytes.
    #[test]
    fn known_size_void_skips_payload() {
        // 0xEC 0x82 0x00 0x00      -> Void, size = 2, 2 bytes of padding
        // 0xE7 0x81 0x2A           -> Timestamp = 42
        let data = vec![0xEC, 0x82, 0x00, 0x00, 0xE7, 0x81, 0x2A];
        let mut it = EbmlIterator::new_endless(0, ebml(data));

        let void = pollster::block_on(it.next()).expect("void element");
        assert_eq!(void.id, ID_VOID);
        assert_eq!(void.size, 4); // 2-byte header + 2-byte payload

        let ts = pollster::block_on(it.next()).expect("timestamp element");
        assert!(matches!(ts.payload, EbmlPayload::UnsignedInt(42)));
    }

    /// The "unknown size" sentinel must be recognised across VINT widths, not
    /// just the 1-octet 0xFF case.
    #[test]
    fn unknown_size_detected_for_multi_octet_vints() {
        // 1-octet: 0xFF
        let (len, size) = pollster::block_on(ebml(vec![0xFF]).read_variable_size_data_size(0));
        assert_eq!((len, size), (1, None));

        // 2-octet: 0x7F 0xFF
        let (len, size) =
            pollster::block_on(ebml(vec![0x7F, 0xFF]).read_variable_size_data_size(0));
        assert_eq!((len, size), (2, None));

        // 2-octet, NOT all ones -> a real value (0x3FFE = 16382)
        let (len, size) =
            pollster::block_on(ebml(vec![0x7F, 0xFE]).read_variable_size_data_size(0));
        assert_eq!((len, size), (2, Some(16382)));
    }
}
