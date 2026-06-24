//! Block → sample extraction: parses (Simple)Block payloads, unlaces frames,
//! reconstructs DTS/composition offsets for reordered video, and converts subtitle
//! blocks to WebVTT cues. Pure, in-memory operations on already-read block bytes.

use crate::fmp4::Sample;

/// One MKV block after parsing: a relative timecode, sync flag, and one or more
/// (laced) frame payloads in storage = decode order.
#[derive(Debug, Clone)]
pub struct BlockFrames {
    pub track_number: u64,
    pub rel_timecode: i16,
    pub is_keyframe: bool,
    pub frames: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lacing {
    None,
    Xiph,
    Ebml,
    Fixed,
}

fn lacing_from_flags(flags: u8) -> Lacing {
    match (flags & 0b0000_0110) >> 1 {
        0b00 => Lacing::None,
        0b01 => Lacing::Xiph,
        0b11 => Lacing::Ebml,
        _ => Lacing::Fixed,
    }
}

/// Read an unsigned EBML variable-length integer (marker bit removed).
fn read_vint(buf: &[u8], pos: usize) -> Option<(u64, usize)> {
    let first = *buf.get(pos)?;
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if pos + len > buf.len() {
        return None;
    }
    let mut value = (first as u64) & (0xff >> len);
    for i in 1..len {
        value = (value << 8) | buf[pos + i] as u64;
    }
    Some((value, len))
}

/// Read a signed EBML lace-size delta (range-shifted vint).
fn read_svint(buf: &[u8], pos: usize) -> Option<(i64, usize)> {
    let (value, len) = read_vint(buf, pos)?;
    let bias = (1i64 << (7 * len - 1)) - 1;
    Some((value as i64 - bias, len))
}

/// Parse a block payload (the bytes *after* the element header). Works for both
/// `SimpleBlock` and the `Block` inside a `BlockGroup`; for the latter pass the
/// keyframe flag determined by the absence of a `ReferenceBlock`.
pub fn parse_block(payload: &[u8], simple_block: bool, group_is_keyframe: bool) -> Option<BlockFrames> {
    let (track_number, mut pos) = read_vint(payload, 0)?;

    let rel_timecode = i16::from_be_bytes([*payload.get(pos)?, *payload.get(pos + 1)?]);
    pos += 2;

    let flags = *payload.get(pos)?;
    pos += 1;

    let is_keyframe = if simple_block {
        flags & 0b1000_0000 != 0
    } else {
        group_is_keyframe
    };

    let lacing = lacing_from_flags(flags);
    let frames = if lacing == Lacing::None {
        vec![payload[pos..].to_vec()]
    } else {
        unlace(payload, pos, lacing)?
    };

    Some(BlockFrames {
        track_number,
        rel_timecode,
        is_keyframe,
        frames,
    })
}

fn unlace(payload: &[u8], mut pos: usize, lacing: Lacing) -> Option<Vec<Vec<u8>>> {
    let frame_count = *payload.get(pos)? as usize + 1;
    pos += 1;

    let mut sizes: Vec<usize> = Vec::with_capacity(frame_count);
    match lacing {
        Lacing::Fixed => {
            let remaining = payload.len() - pos;
            if remaining % frame_count != 0 {
                return None;
            }
            let each = remaining / frame_count;
            sizes = vec![each; frame_count];
        }
        Lacing::Xiph => {
            for _ in 0..frame_count - 1 {
                let mut size = 0usize;
                loop {
                    let b = *payload.get(pos)?;
                    pos += 1;
                    size += b as usize;
                    if b != 255 {
                        break;
                    }
                }
                sizes.push(size);
            }
            // Last frame takes the remainder.
            let used: usize = sizes.iter().sum();
            sizes.push(payload.len().checked_sub(pos + used)?);
        }
        Lacing::Ebml => {
            let (first, len) = read_vint(payload, pos)?;
            pos += len;
            let mut current = first as i64;
            sizes.push(current as usize);
            for _ in 0..frame_count - 2 {
                let (delta, len) = read_svint(payload, pos)?;
                pos += len;
                current += delta;
                if current < 0 {
                    return None;
                }
                sizes.push(current as usize);
            }
            // Last frame takes the remainder.
            if frame_count >= 2 {
                let used: usize = sizes.iter().sum();
                sizes.push(payload.len().checked_sub(pos + used)?);
            }
        }
        Lacing::None => unreachable!(),
    }

    let mut frames = Vec::with_capacity(frame_count);
    for size in sizes {
        let end = pos.checked_add(size)?;
        if end > payload.len() {
            return None;
        }
        frames.push(payload[pos..end].to_vec());
        pos = end;
    }
    Some(frames)
}

/// A decoded-order frame with an absolute presentation time, ready for muxing.
#[derive(Debug, Clone)]
pub struct TimedFrame {
    /// Absolute presentation time in MKV ticks (TimestampScale units).
    pub pts_ticks: i64,
    pub data: Vec<u8>,
    pub is_keyframe: bool,
}

/// Build video samples from decode-ordered frames, reconstructing a monotonic DTS
/// for reordered (B-frame) streams.
///
/// MKV stores only PTS (in decode order). We reconstruct DTS with the standard
/// "DTS = sorted PTS" method: the multiset of decode-order DTS equals the multiset
/// of PTS, assigned ascending. This makes DTS strictly monotonic and — crucially —
/// makes per-sample durations the *exact* sorted-PTS deltas, so a segment accrues no
/// rounding drift even at a coarse (millisecond) timescale. Composition offsets
/// `cts[i] = pts[i] - dts[i]` may be negative; the caller writes a version-1 `trun`
/// with signed offsets. `frame_dur_ticks` only sizes the final sample (no successor).
/// Returns `(base_media_decode_time, samples)` in MKV ticks.
pub fn video_samples(frames: &[TimedFrame], frame_dur_ticks: i64) -> (u64, Vec<Sample>) {
    if frames.is_empty() {
        return (0, Vec::new());
    }
    let mut sorted_pts: Vec<i64> = frames.iter().map(|f| f.pts_ticks).collect();
    sorted_pts.sort_unstable();
    let base = sorted_pts[0];
    let fallback_dur = frame_dur_ticks.max(1);

    let samples = frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let dts = sorted_pts[i];
            let duration = if i + 1 < sorted_pts.len() {
                (sorted_pts[i + 1] - dts).max(0)
            } else {
                fallback_dur
            };
            Sample {
                data: f.data.clone(),
                duration: duration as u32,
                cts_offset: (f.pts_ticks - dts) as i32,
                is_sync: f.is_keyframe,
            }
        })
        .collect();

    (base.max(0) as u64, samples)
}

/// Build audio samples. Audio is never reordered (cts = 0) and uses a sample-count
/// timeline: `samples_per_frame` per frame in the sample-rate timescale, with the
/// segment anchored at the first frame's time converted to samples. Returns
/// `(base_media_decode_time_in_samples, samples)`.
pub fn audio_samples(
    frames: &[TimedFrame],
    samples_per_frame: u32,
    sample_rate: u32,
    timestamp_scale_ns: u64,
) -> (u64, Vec<Sample>) {
    if frames.is_empty() {
        return (0, Vec::new());
    }
    // ticks → seconds → samples. ticks * timestamp_scale_ns / 1e9 * sample_rate.
    let first_ticks = frames[0].pts_ticks.max(0) as u128;
    let base = first_ticks * timestamp_scale_ns as u128 * sample_rate as u128 / 1_000_000_000u128;

    let samples = frames
        .iter()
        .map(|f| Sample {
            data: f.data.clone(),
            duration: samples_per_frame,
            cts_offset: 0,
            is_sync: true,
        })
        .collect();

    (base as u64, samples)
}

// ============================================================================
// Subtitles
// ============================================================================

/// One subtitle cue with millisecond start/end and UTF-8 text.
#[derive(Debug, Clone)]
pub struct SubtitleCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Assemble a WebVTT document from cues. `S_TEXT/UTF8` (SubRip) text passes through
/// unchanged except for the `,`→`.` timestamp form handled here at the cue level.
pub fn cues_to_webvtt(cues: &[SubtitleCue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_vtt_time(cue.start_ms),
            format_vtt_time(cue.end_ms),
            cue.text.trim_end()
        ));
    }
    out
}

fn format_vtt_time(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unlaced_simpleblock() {
        // track 1 (0x81), timecode 0x0000, flags 0x80 (keyframe, no lacing), data.
        let payload = [0x81, 0x00, 0x00, 0x80, 0xaa, 0xbb, 0xcc];
        let b = parse_block(&payload, true, false).unwrap();
        assert_eq!(b.track_number, 1);
        assert_eq!(b.rel_timecode, 0);
        assert!(b.is_keyframe);
        assert_eq!(b.frames, vec![vec![0xaa, 0xbb, 0xcc]]);
    }

    #[test]
    fn parses_fixed_lacing() {
        // flags 0x04 (fixed lacing), num_frames-1=1 (2 frames), 4 bytes → 2 each.
        let payload = [0x81, 0x00, 0x00, 0x04, 0x01, 1, 2, 3, 4];
        let b = parse_block(&payload, true, false).unwrap();
        assert_eq!(b.frames, vec![vec![1, 2], vec![3, 4]]);
    }

    #[test]
    fn parses_ebml_lacing() {
        // flags 0x06 (EBML lacing), num-1=1 (2 frames), first size vint 0x82 (=2),
        // then 2 + remainder(3) bytes.
        let payload = [0x81, 0x00, 0x00, 0x06, 0x01, 0x82, 1, 2, 3, 4, 5];
        let b = parse_block(&payload, true, false).unwrap();
        assert_eq!(b.frames, vec![vec![1, 2], vec![3, 4, 5]]);
    }

    #[test]
    fn parses_xiph_lacing() {
        // 3 frames; xiph sizes: frame0=1, frame1=2, frame2=remainder.
        // flags 0x02 (xiph), num-1=2, size0=0x01, size1=0x02, then 1+2+rest bytes.
        let payload = [0x81, 0x00, 0x00, 0x02, 0x02, 0x01, 0x02, 10, 20, 21, 30, 31, 32];
        let b = parse_block(&payload, true, false).unwrap();
        assert_eq!(b.frames, vec![vec![10], vec![20, 21], vec![30, 31, 32]]);
    }

    #[test]
    fn reconstructs_dts_for_reordered_video() {
        // Decode order I,P,B,B with PTS 0,3,1,2 (display 0,1,2,3).
        let frames = vec![
            TimedFrame { pts_ticks: 0, data: vec![0], is_keyframe: true },
            TimedFrame { pts_ticks: 3, data: vec![1], is_keyframe: false },
            TimedFrame { pts_ticks: 1, data: vec![2], is_keyframe: false },
            TimedFrame { pts_ticks: 2, data: vec![3], is_keyframe: false },
        ];
        let (base, samples) = video_samples(&frames, 1);
        assert_eq!(base, 0);
        // DTS = sorted PTS = [0,1,2,3] in decode order; monotonic, durations exact.
        let mut dts = 0i64;
        for (i, s) in samples.iter().enumerate() {
            // presentation time = dts + cts must equal the original PTS.
            assert_eq!(dts + s.cts_offset as i64, frames[i].pts_ticks);
            dts += s.duration as i64;
        }
        // Durations sum to the full presentation span (no drift): 1+1+1+1 = 4.
        let total: u32 = samples.iter().map(|s| s.duration).sum();
        assert_eq!(total, 4);
    }

    #[test]
    fn webvtt_timestamps_use_dot_separator() {
        let cues = vec![SubtitleCue { start_ms: 61_500, end_ms: 65_000, text: "Hi".into() }];
        let vtt = cues_to_webvtt(&cues);
        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("00:01:01.500 --> 00:01:05.000"));
    }
}
