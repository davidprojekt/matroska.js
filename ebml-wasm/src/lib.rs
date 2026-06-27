pub mod fetch_source;
pub mod ebml;
pub mod matroska_data;
pub mod fs_source;
pub mod mem_source;

use wasm_bindgen::prelude::*;
use std::collections::HashMap;
use ebml_spec::parse_xml;
use crate::ebml::{Ebml, EbmlElement, EbmlIterator, EbmlPayload, EbmlSource, Size};
use crate::matroska_data::element_id_type_map;
use crate::fetch_source::FetchSource;


pub fn create_sample_instance() -> Ebml<FetchSource> {
    let file_source = FetchSource::new("http://127.0.0.1:8501/example/sample.mkv".to_string());

    Ebml::new(file_source, element_id_type_map())
}

pub fn create_instance(url: String) -> Ebml<FetchSource> {
    let file_source = FetchSource::new(url);

    Ebml::new(file_source, element_id_type_map())
}

#[wasm_bindgen]
struct FetchSourceEbml(Ebml<FetchSource>);

#[wasm_bindgen]
impl FetchSourceEbml {
    pub fn new(url: String) -> Self {
        Self(create_instance(url))
    }

}

#[wasm_bindgen]
struct EbmlReader(EbmlIterator<FetchSource>);

#[wasm_bindgen]
impl EbmlReader {
    pub fn new(offset: u64, end: u64, ebml: FetchSourceEbml) -> Self {
        Self(EbmlIterator::new(offset, end, ebml.0))
    }
    pub fn new_endless(offset: u64, ebml: FetchSourceEbml) -> Self {
        Self(EbmlIterator::new_endless(offset, ebml.0))
    }

    pub async fn next(&mut self) -> Option<WrappedEbmlElement> {
        self.0.next().await.map(|e| WrappedEbmlElement(e))
    }

    pub async fn read_bytes(&mut self, range: Range) -> Box<[u8]> {
        self.0.read_range(range.start, range.end).await
    }

    pub fn split(&self, offset: u64) -> Self {
        let mut clone = Self(self.0.clone());

        clone.0.current = offset;
        clone.0.end = None;

        clone
    }

    pub fn seek(&mut self, offset: u64) {

        self.0.current = offset;
        self.0.end = None;
    }

}

#[wasm_bindgen]
pub struct WrappedEbmlElement(EbmlElement<FetchSource>);


#[wasm_bindgen]
impl WrappedEbmlElement {

    pub fn get_id(&self) -> u64 {
        self.0.id
    }
    pub fn get_offset(&self) -> u64 {
        self.0.offset
    }
    pub fn get_size(&self) -> u64 {
        self.0.size
    }
    pub fn get_payload(&self) -> WrappedEbmlPayload {
        match &self.0.payload {
            EbmlPayload::Master(v) => WrappedEbmlPayload::Master(EbmlReader(v.clone())),
            EbmlPayload::UnsignedInt(v) => WrappedEbmlPayload::UnsignedInt(v.clone()),
            EbmlPayload::SignedInt(v) => WrappedEbmlPayload::SignedInt(v.clone()),
            EbmlPayload::Float(v) => WrappedEbmlPayload::Float(v.clone()),
            EbmlPayload::Date(v) => WrappedEbmlPayload::Date(v.clone()),
            EbmlPayload::String(v) => WrappedEbmlPayload::String(v.clone()),
            EbmlPayload::Binary(v) => WrappedEbmlPayload::Binary(Range::new(v.clone())),
            EbmlPayload::Void => WrappedEbmlPayload::Void(Nothing{}),
            EbmlPayload::Unknown(v) => WrappedEbmlPayload::Unknown(Range::new(v.clone())),
            EbmlPayload::Invalid(v) => WrappedEbmlPayload::Invalid(v.map(|v| Range::new(v))),
        }
    }

    pub fn get_payload_type(&self) -> String {
        match &self.0.payload {
            EbmlPayload::Master(_) => "Master".to_string(),
            EbmlPayload::UnsignedInt(_) => "UnsignedInt".to_string(),
            EbmlPayload::SignedInt(_) => "SignedInt".to_string(),
            EbmlPayload::Float(_) => "Float".to_string(),
            EbmlPayload::Date(_) => "Date".to_string(),
            EbmlPayload::String(_) => "String".to_string(),
            EbmlPayload::Binary(_) => "Binary".to_string(),
            EbmlPayload::Void => "Void".to_string(),
            EbmlPayload::Unknown(_) => "Unknown".to_string(),
            EbmlPayload::Invalid(_) => "Invalid".to_string(),
        }
    }

}


#[wasm_bindgen]
pub struct Range {
    start: u64,
    end: u64,
}

impl Range {
    pub fn new(tupl: (u64, u64)) -> Self {
        Self {
            start: tupl.0,
            end: tupl.1
        }
    }
}

#[wasm_bindgen]
pub struct Nothing { }

#[wasm_bindgen]
pub enum WrappedEbmlPayload {
    Master(EbmlReader),
    UnsignedInt(u64),
    SignedInt(i64),
    Float(f64),
    Date(i64),
    String(String),
    Binary(Range),
    Void(Nothing),

    Unknown(Range),
    Invalid(Option<Range>),
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let elements = element_id_type_map();

    }
}
