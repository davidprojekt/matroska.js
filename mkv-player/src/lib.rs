//! MKV → fragmented-MP4 remuxer and player facade.
//!
//! Builds on the `ebml-wasm` parser crate: it consumes the EBML/Matroska reader and
//! byte sources from there, and turns a Matroska Segment into MSE-appendable fMP4
//! (init + media segments), exposing the [`player::Demuxer`] and the `wasm-bindgen`
//! `MatroskaPlayer` facade.
//!
//! Licensed under AGPL-3.0-only (see ../LICENSE.txt). The parser core it depends on
//! (`ebml-wasm`, `ebml-spec`) is MIT-licensed and can be used independently.

pub mod fmp4;
pub mod remux;
pub mod index;
pub mod track;
pub mod stream_source;
pub mod player;
