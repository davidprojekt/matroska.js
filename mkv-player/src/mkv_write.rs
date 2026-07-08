//! Minimal EBML/Matroska writer.
//!
//! Just enough to emit a self-contained, single-audio-track Matroska chunk
//! (EBML header + Segment{ Info + Tracks + Cluster(s) }) for one time window. The
//! chunk is handed to ffmpeg.wasm in the browser to transcode audio codecs that
//! MSE can't decode natively (DTS, TrueHD, Vorbis, PCM, …). `ebml-wasm` is a
//! parser only, so the encoder lives here.
//!
//! The chunk's timeline is **zero-anchored** at its first frame: ffmpeg's MP4
//! muxer normalizes the first output packet to PTS 0 regardless of input start,
//! so absolute timestamps can't survive the transcode. Instead the caller is told
//! the chunk's true start time separately and re-places it on the MSE timeline with
//! `SourceBuffer.timestampOffset`. Each Cluster is re-anchored at its own first
//! frame so per-block relative timecodes stay within the MKV `i16` range.

use crate::remux::TimedFrame;
use ebml_wasm::matroska_data::{
    ID_AUDIO, ID_BITDEPTH, ID_CHANNELS, ID_CLUSTER, ID_CODECDELAY, ID_CODECID, ID_CODECPRIVATE,
    ID_DOCTYPE, ID_DOCTYPE_READ_VERSION, ID_DOCTYPE_VERSION, ID_EBML, ID_EBMLMAX_IDLENGTH,
    ID_EBMLMAX_SIZE_LENGTH, ID_EBMLREAD_VERSION, ID_EBMLVERSION, ID_FLAGLACING, ID_INFO,
    ID_MUXINGAPP, ID_SAMPLINGFREQUENCY, ID_SEEKPREROLL, ID_SEGMENT, ID_SIMPLEBLOCK,
    ID_TIMESTAMP, ID_TIMESTAMPSCALE, ID_TRACKENTRY, ID_TRACKNUMBER, ID_TRACKS, ID_TRACKTYPE,
    ID_TRACKUID, ID_WRITINGAPP,
};

/// Everything needed to describe the one audio track in the emitted chunk.
pub struct AudioChunkParams<'a> {
    pub timestamp_scale_ns: u64,
    pub codec_id: &'a str,
    pub codec_private: Option<&'a [u8]>,
    pub sample_rate: f64,
    pub channels: u64,
    pub bit_depth: Option<u64>,
    /// `CodecDelay` / `SeekPreroll` in nanoseconds (Opus), copied through verbatim.
    pub codec_delay_ns: Option<u64>,
    pub seek_preroll_ns: Option<u64>,
}

/// Append an element ID. The `ID_*` constants are the *stored* form (their
/// length-descriptor bits are already part of the value), so we just emit the
/// value as its minimal big-endian bytes.
fn write_id(out: &mut Vec<u8>, id: u64) {
    let n = if id <= 0xFF {
        1
    } else if id <= 0xFFFF {
        2
    } else if id <= 0xFF_FFFF {
        3
    } else {
        4
    };
    for i in (0..n).rev() {
        out.push((id >> (8 * i)) as u8);
    }
}

/// Append an EBML variable-length integer (marker bit included). Used both for
/// element sizes and for a Block's track-number field. Picks the smallest width
/// whose value range can hold `value` (the all-ones encoding is reserved).
fn write_vint(out: &mut Vec<u8>, value: u64) {
    let mut n = 1usize;
    while n < 8 && value >= (1u64 << (7 * n)) - 1 {
        n += 1;
    }
    let v = (1u64 << (7 * n)) | value; // set the width-marker bit
    for i in (0..n).rev() {
        out.push((v >> (8 * i)) as u8);
    }
}

/// Write a complete element: id, size (vint), then payload.
fn elem(out: &mut Vec<u8>, id: u64, payload: &[u8]) {
    write_id(out, id);
    write_vint(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

/// Write an unsigned-integer element using the minimal big-endian byte count
/// (at least one byte, so zero is encoded as a single `0x00`).
fn elem_uint(out: &mut Vec<u8>, id: u64, value: u64) {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len() - 1);
    elem(out, id, &bytes[first..]);
}

fn elem_string(out: &mut Vec<u8>, id: u64, s: &str) {
    elem(out, id, s.as_bytes());
}

/// Write an 8-byte (double-precision) float element, as Matroska uses for
/// `SamplingFrequency`.
fn elem_f64(out: &mut Vec<u8>, id: u64, v: f64) {
    elem(out, id, &v.to_be_bytes());
}

fn write_ebml_header(out: &mut Vec<u8>) {
    let mut h = Vec::new();
    elem_uint(&mut h, ID_EBMLVERSION, 1);
    elem_uint(&mut h, ID_EBMLREAD_VERSION, 1);
    elem_uint(&mut h, ID_EBMLMAX_IDLENGTH, 4);
    elem_uint(&mut h, ID_EBMLMAX_SIZE_LENGTH, 8);
    elem_string(&mut h, ID_DOCTYPE, "matroska");
    elem_uint(&mut h, ID_DOCTYPE_VERSION, 4);
    elem_uint(&mut h, ID_DOCTYPE_READ_VERSION, 2);
    elem(out, ID_EBML, &h);
}

/// One SimpleBlock for a single (already-unlaced) frame: track number (vint),
/// signed relative timecode, flags (keyframe, no lacing), then the frame bytes.
fn write_simpleblock(cluster: &mut Vec<u8>, track: u64, rel: i16, data: &[u8]) {
    let mut payload = Vec::with_capacity(data.len() + 4);
    write_vint(&mut payload, track);
    payload.extend_from_slice(&rel.to_be_bytes());
    payload.push(0x80); // keyframe, no lacing — audio frames are independent
    payload.extend_from_slice(data);
    elem(cluster, ID_SIMPLEBLOCK, &payload);
}

/// Emit Cluster elements covering all `frames`, with the whole timeline shifted so
/// the first frame sits at tick 0 (`anchor` = first frame's absolute PTS). A new
/// Cluster is started whenever the next frame would fall outside the `i16`
/// relative-timecode range of the current Cluster, so long windows never overflow.
fn write_clusters(seg: &mut Vec<u8>, frames: &[TimedFrame], anchor: i64) {
    let mut i = 0;
    while i < frames.len() {
        let base = frames[i].pts_ticks - anchor;
        let mut cluster = Vec::new();
        elem_uint(&mut cluster, ID_TIMESTAMP, base.max(0) as u64);
        while i < frames.len() {
            let rel = (frames[i].pts_ticks - anchor) - base;
            if rel < i16::MIN as i64 || rel > i16::MAX as i64 {
                break;
            }
            write_simpleblock(&mut cluster, 1, rel as i16, &frames[i].data);
            i += 1;
        }
        elem(seg, ID_CLUSTER, &cluster);
    }
}

/// Build a self-contained Matroska chunk with one audio track and the given frames.
pub fn build_audio_chunk(p: &AudioChunkParams, frames: &[TimedFrame]) -> Vec<u8> {
    let mut out = Vec::new();
    write_ebml_header(&mut out);

    let mut seg = Vec::new();

    // Info
    let mut info = Vec::new();
    elem_uint(&mut info, ID_TIMESTAMPSCALE, p.timestamp_scale_ns.max(1));
    elem_string(&mut info, ID_MUXINGAPP, "mkv-player");
    elem_string(&mut info, ID_WRITINGAPP, "mkv-player");
    elem(&mut seg, ID_INFO, &info);

    // Tracks → one audio TrackEntry
    let mut track = Vec::new();
    elem_uint(&mut track, ID_TRACKNUMBER, 1);
    elem_uint(&mut track, ID_TRACKUID, 1);
    elem_uint(&mut track, ID_TRACKTYPE, 2); // 2 = audio
    elem_uint(&mut track, ID_FLAGLACING, 0);
    elem_string(&mut track, ID_CODECID, p.codec_id);
    if let Some(cp) = p.codec_private {
        elem(&mut track, ID_CODECPRIVATE, cp);
    }
    if let Some(d) = p.codec_delay_ns {
        elem_uint(&mut track, ID_CODECDELAY, d);
    }
    if let Some(s) = p.seek_preroll_ns {
        elem_uint(&mut track, ID_SEEKPREROLL, s);
    }
    let mut audio = Vec::new();
    elem_f64(&mut audio, ID_SAMPLINGFREQUENCY, p.sample_rate);
    elem_uint(&mut audio, ID_CHANNELS, p.channels.max(1));
    if let Some(b) = p.bit_depth {
        elem_uint(&mut audio, ID_BITDEPTH, b);
    }
    elem(&mut track, ID_AUDIO, &audio);

    let mut tracks = Vec::new();
    elem(&mut tracks, ID_TRACKENTRY, &track);
    elem(&mut seg, ID_TRACKS, &tracks);

    // Clusters (zero-anchored at the first frame).
    let anchor = frames.first().map(|f| f.pts_ticks).unwrap_or(0);
    write_clusters(&mut seg, frames, anchor);

    elem(&mut out, ID_SEGMENT, &seg);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_widths_and_marker() {
        let mut b = Vec::new();
        write_vint(&mut b, 1);
        assert_eq!(b, vec![0x81]); // 1-byte: marker 0x80 | 1
        b.clear();
        write_vint(&mut b, 126);
        assert_eq!(b, vec![0xFE]); // largest 1-byte value
        b.clear();
        write_vint(&mut b, 127);
        assert_eq!(b, vec![0x40, 0x7F]); // 127 needs 2 bytes (0x7F is the 1-byte reserved value)
        b.clear();
        write_vint(&mut b, 300);
        assert_eq!(b, vec![0x41, 0x2C]); // 2-byte: 0x4000 | 300
    }

    #[test]
    fn ids_emit_minimal_bytes() {
        let mut b = Vec::new();
        write_id(&mut b, ID_TIMESTAMP); // 0xE7, 1 byte
        assert_eq!(b, vec![0xE7]);
        b.clear();
        write_id(&mut b, ID_EBML); // 0x1A45DFA3, 4 bytes
        assert_eq!(b, vec![0x1A, 0x45, 0xDF, 0xA3]);
        b.clear();
        write_id(&mut b, ID_SEGMENT); // 0x18538067, 4 bytes
        assert_eq!(b, vec![0x18, 0x53, 0x80, 0x67]);
    }

    #[test]
    fn chunk_starts_with_ebml_magic_and_holds_a_segment() {
        let frames = vec![
            TimedFrame { pts_ticks: 1000, data: vec![1, 2, 3], is_keyframe: true },
            TimedFrame { pts_ticks: 1020, data: vec![4, 5, 6], is_keyframe: true },
        ];
        let p = AudioChunkParams {
            timestamp_scale_ns: 1_000_000,
            codec_id: "A_DTS",
            codec_private: None,
            sample_rate: 48000.0,
            channels: 6,
            bit_depth: None,
            codec_delay_ns: None,
            seek_preroll_ns: None,
        };
        let chunk = build_audio_chunk(&p, &frames);
        // EBML magic.
        assert_eq!(&chunk[0..4], &[0x1A, 0x45, 0xDF, 0xA3]);
        // Segment ID appears after the header.
        assert!(chunk
            .windows(4)
            .any(|w| w == [0x18, 0x53, 0x80, 0x67]));
        // The raw frame payloads are carried verbatim.
        assert!(chunk.windows(3).any(|w| w == [1, 2, 3]));
        assert!(chunk.windows(3).any(|w| w == [4, 5, 6]));
    }

    #[test]
    fn long_window_splits_clusters_on_i16_overflow() {
        // Two frames >32s apart at 1ms scale must land in separate clusters.
        let frames = vec![
            TimedFrame { pts_ticks: 0, data: vec![0xAA], is_keyframe: true },
            TimedFrame { pts_ticks: 40_000, data: vec![0xBB], is_keyframe: true },
        ];
        let p = AudioChunkParams {
            timestamp_scale_ns: 1_000_000,
            codec_id: "A_DTS",
            codec_private: None,
            sample_rate: 48000.0,
            channels: 2,
            bit_depth: None,
            codec_delay_ns: None,
            seek_preroll_ns: None,
        };
        let chunk = build_audio_chunk(&p, &frames);
        let clusters = chunk.windows(4).filter(|w| *w == [0x1F, 0x43, 0xB6, 0x75]).count();
        assert_eq!(clusters, 2);
    }
}
