//! Minimal fragmented-MP4 (fMP4) writer for Media Source Extensions.
//!
//! Pure ISO-BMFF box serialization — no I/O. One [`TrackConfig`] drives a single
//! track's init segment (`ftyp`+`moov`) and any number of media segments
//! (`moof`+`mdat`). Unlike a live muxer, the caller supplies an explicit absolute
//! `base_media_decode_time` per media segment so segments are position-independent
//! and seeking is just "emit the right segment".
//!
//! Structurally modeled on muxide's permissively-licensed `src/fragmented.rs`,
//! extended for audio tracks and arbitrary track ids / decode times.

/// What kind of track this is, with the dimensions needed for its sample entry.
#[derive(Debug, Clone)]
pub enum MediaKind {
    Video { width: u16, height: u16 },
    Audio { sample_rate: u32, channels: u16 },
}

/// Codec configuration, carrying the raw MKV `CodecPrivate` where the box payload
/// is identical to it (avcC/hvcC/av1C) or the bytes needed to build the box.
#[derive(Debug, Clone)]
pub enum CodecConfig {
    /// `CodecPrivate` is the AVCDecoderConfigurationRecord verbatim.
    Avc(Vec<u8>),
    /// `CodecPrivate` is the HEVCDecoderConfigurationRecord verbatim.
    Hevc(Vec<u8>),
    /// `CodecPrivate` is the AV1CodecConfigurationRecord verbatim.
    Av1(Vec<u8>),
    /// VP9: optional vpcC payload (often absent in MKV).
    Vp9(Option<Vec<u8>>),
    /// AAC: `CodecPrivate` is the AudioSpecificConfig.
    Aac(Vec<u8>),
    /// Opus: `CodecPrivate` is the OpusHead.
    Opus(Vec<u8>),
    /// AC-3: `CodecPrivate` (may be empty; dac3 then defaulted).
    Ac3(Vec<u8>),
}

impl CodecConfig {
    /// FourCC for the sample entry box.
    fn fourcc(&self) -> &'static [u8; 4] {
        match self {
            CodecConfig::Avc(_) => b"avc1",
            CodecConfig::Hevc(_) => b"hvc1",
            CodecConfig::Av1(_) => b"av01",
            CodecConfig::Vp9(_) => b"vp09",
            CodecConfig::Aac(_) => b"mp4a",
            CodecConfig::Opus(_) => b"Opus",
            CodecConfig::Ac3(_) => b"ac-3",
        }
    }

    fn is_video(&self) -> bool {
        matches!(
            self,
            CodecConfig::Avc(_) | CodecConfig::Hevc(_) | CodecConfig::Av1(_) | CodecConfig::Vp9(_)
        )
    }

    /// The codec configuration box (avcC/hvcC/av1C/vpcC/esds/dOps/dac3).
    fn config_box(&self, kind: &MediaKind) -> Vec<u8> {
        match self {
            CodecConfig::Avc(cp) => boxed(b"avcC", cp),
            CodecConfig::Hevc(cp) => boxed(b"hvcC", cp),
            CodecConfig::Av1(cp) => boxed(b"av1C", cp),
            CodecConfig::Vp9(cp) => boxed(b"vpcC", cp.as_deref().unwrap_or(&[])),
            CodecConfig::Aac(asc) => build_esds(asc),
            CodecConfig::Opus(head) => build_dops(head, kind),
            CodecConfig::Ac3(cp) => boxed(b"dac3", cp),
        }
    }
}

/// Everything needed to mux one track.
#[derive(Debug, Clone)]
pub struct TrackConfig {
    pub track_id: u32,
    pub timescale: u32,
    /// Total duration in `timescale` units (0 if unknown).
    pub duration: u64,
    pub language: String,
    pub kind: MediaKind,
    pub codec: CodecConfig,
}

/// A single decoded-order sample to place in a media segment.
#[derive(Debug, Clone)]
pub struct Sample {
    pub data: Vec<u8>,
    /// Duration in `timescale` units.
    pub duration: u32,
    /// Composition offset `PTS - DTS` in `timescale` units (0 for audio).
    pub cts_offset: i32,
    pub is_sync: bool,
}

// ============================================================================
// Box primitives
// ============================================================================

fn boxed(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8 + payload.len()) as u32;
    let mut buf = Vec::with_capacity(size as usize);
    buf.extend_from_slice(&size.to_be_bytes());
    buf.extend_from_slice(typ);
    buf.extend_from_slice(payload);
    buf
}

fn boxed_concat(typ: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
    let payload_len: usize = parts.iter().map(|p| p.len()).sum();
    let mut payload = Vec::with_capacity(payload_len);
    for p in parts {
        payload.extend_from_slice(p);
    }
    boxed(typ, &payload)
}

/// ISO 639-2/T language packed into 15 bits (3 chars, each `c - 0x60`).
fn pack_language(language: &str) -> [u8; 2] {
    let bytes = language.as_bytes();
    let c = |i: usize| -> u16 {
        let ch = bytes.get(i).copied().unwrap_or(b'u');
        ((ch as u16).wrapping_sub(0x60)) & 0x1f
    };
    let packed = (c(0) << 10) | (c(1) << 5) | c(2);
    packed.to_be_bytes()
}

// ============================================================================
// Init segment (ftyp + moov)
// ============================================================================

pub fn build_init_segment(cfg: &TrackConfig) -> Vec<u8> {
    let mut out = build_ftyp();
    out.extend_from_slice(&build_moov(cfg));
    out
}

fn build_ftyp() -> Vec<u8> {
    boxed_concat(b"ftyp", &[b"iso5", &0u32.to_be_bytes(), b"iso5", b"iso6", b"mp41"])
}

fn build_moov(cfg: &TrackConfig) -> Vec<u8> {
    let mvhd = build_mvhd(cfg);
    let trak = build_trak(cfg);
    let mvex = build_mvex(cfg.track_id);
    boxed_concat(b"moov", &[&mvhd, &trak, &mvex])
}

fn build_mvhd(cfg: &TrackConfig) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    p.extend_from_slice(&0u32.to_be_bytes()); // creation time
    p.extend_from_slice(&0u32.to_be_bytes()); // modification time
    p.extend_from_slice(&cfg.timescale.to_be_bytes());
    p.extend_from_slice(&(cfg.duration as u32).to_be_bytes());
    p.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    p.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    p.extend_from_slice(&[0u8; 10]); // reserved
    p.extend_from_slice(&unity_matrix());
    p.extend_from_slice(&[0u8; 24]); // pre-defined
    p.extend_from_slice(&(cfg.track_id + 1).to_be_bytes()); // next track id
    boxed(b"mvhd", &p)
}

fn unity_matrix() -> [u8; 36] {
    let mut m = [0u8; 36];
    m[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[16..20].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[32..36].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    m
}

fn build_mvex(track_id: u32) -> Vec<u8> {
    let mut trex = Vec::new();
    trex.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    trex.extend_from_slice(&track_id.to_be_bytes());
    trex.extend_from_slice(&1u32.to_be_bytes()); // default sample description index
    trex.extend_from_slice(&0u32.to_be_bytes()); // default sample duration
    trex.extend_from_slice(&0u32.to_be_bytes()); // default sample size
    trex.extend_from_slice(&0u32.to_be_bytes()); // default sample flags
    let trex = boxed(b"trex", &trex);
    boxed(b"mvex", &trex)
}

fn build_trak(cfg: &TrackConfig) -> Vec<u8> {
    let tkhd = build_tkhd(cfg);
    let mdia = build_mdia(cfg);
    boxed_concat(b"trak", &[&tkhd, &mdia])
}

fn build_tkhd(cfg: &TrackConfig) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0x0000_0003u32.to_be_bytes()); // version 0, flags: enabled + in movie
    p.extend_from_slice(&0u32.to_be_bytes()); // creation time
    p.extend_from_slice(&0u32.to_be_bytes()); // modification time
    p.extend_from_slice(&cfg.track_id.to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes()); // reserved
    p.extend_from_slice(&(cfg.duration as u32).to_be_bytes());
    p.extend_from_slice(&[0u8; 8]); // reserved
    p.extend_from_slice(&0u16.to_be_bytes()); // layer
    p.extend_from_slice(&0u16.to_be_bytes()); // alternate group
    let volume: u16 = if cfg.codec.is_video() { 0 } else { 0x0100 };
    p.extend_from_slice(&volume.to_be_bytes());
    p.extend_from_slice(&0u16.to_be_bytes()); // reserved
    p.extend_from_slice(&unity_matrix());
    let (w, h) = match cfg.kind {
        MediaKind::Video { width, height } => (width as u32, height as u32),
        MediaKind::Audio { .. } => (0, 0),
    };
    p.extend_from_slice(&(w << 16).to_be_bytes());
    p.extend_from_slice(&(h << 16).to_be_bytes());
    boxed(b"tkhd", &p)
}

fn build_mdia(cfg: &TrackConfig) -> Vec<u8> {
    let mdhd = build_mdhd(cfg);
    let hdlr = build_hdlr(&cfg.kind);
    let minf = build_minf(cfg);
    boxed_concat(b"mdia", &[&mdhd, &hdlr, &minf])
}

fn build_mdhd(cfg: &TrackConfig) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    p.extend_from_slice(&0u32.to_be_bytes()); // creation time
    p.extend_from_slice(&0u32.to_be_bytes()); // modification time
    p.extend_from_slice(&cfg.timescale.to_be_bytes());
    p.extend_from_slice(&(cfg.duration as u32).to_be_bytes());
    p.extend_from_slice(&pack_language(&cfg.language));
    p.extend_from_slice(&0u16.to_be_bytes()); // pre-defined
    boxed(b"mdhd", &p)
}

fn build_hdlr(kind: &MediaKind) -> Vec<u8> {
    let (handler, name): (&[u8; 4], &[u8]) = match kind {
        MediaKind::Video { .. } => (b"vide", b"VideoHandler\0"),
        MediaKind::Audio { .. } => (b"soun", b"SoundHandler\0"),
    };
    let mut p = Vec::new();
    p.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    p.extend_from_slice(&0u32.to_be_bytes()); // pre-defined
    p.extend_from_slice(handler);
    p.extend_from_slice(&[0u8; 12]); // reserved
    p.extend_from_slice(name);
    boxed(b"hdlr", &p)
}

fn build_minf(cfg: &TrackConfig) -> Vec<u8> {
    let media_header = match cfg.kind {
        MediaKind::Video { .. } => build_vmhd(),
        MediaKind::Audio { .. } => build_smhd(),
    };
    let dinf = build_dinf();
    let stbl = build_stbl(cfg);
    boxed_concat(b"minf", &[&media_header, &dinf, &stbl])
}

fn build_vmhd() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0x0000_0001u32.to_be_bytes()); // version 0, flags 1
    p.extend_from_slice(&[0u8; 8]); // graphics mode + opcolor
    boxed(b"vmhd", &p)
}

fn build_smhd() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    p.extend_from_slice(&0u16.to_be_bytes()); // balance
    p.extend_from_slice(&0u16.to_be_bytes()); // reserved
    boxed(b"smhd", &p)
}

fn build_dinf() -> Vec<u8> {
    let url = boxed(b"url ", &0x0000_0001u32.to_be_bytes()); // self-contained
    let mut dref = Vec::new();
    dref.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    dref.extend_from_slice(&1u32.to_be_bytes()); // entry count
    dref.extend_from_slice(&url);
    let dref = boxed(b"dref", &dref);
    boxed(b"dinf", &dref)
}

fn build_stbl(cfg: &TrackConfig) -> Vec<u8> {
    let stsd = build_stsd(cfg);
    // Empty sample tables — the real per-sample data lives in each moof's trun.
    let stts = boxed(b"stts", &[0u8; 8]); // version+flags + entry_count
    let stsc = boxed(b"stsc", &[0u8; 8]); // version+flags + entry_count
    let stsz = boxed(b"stsz", &[0u8; 12]); // version+flags + sample_size + sample_count
    let stco = boxed(b"stco", &[0u8; 8]); // version+flags + entry_count
    boxed_concat(b"stbl", &[&stsd, &stts, &stsc, &stsz, &stco])
}

fn build_stsd(cfg: &TrackConfig) -> Vec<u8> {
    let entry = build_sample_entry(cfg);
    let mut p = Vec::new();
    p.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    p.extend_from_slice(&1u32.to_be_bytes()); // entry count
    p.extend_from_slice(&entry);
    boxed(b"stsd", &p)
}

fn build_sample_entry(cfg: &TrackConfig) -> Vec<u8> {
    let fourcc = cfg.codec.fourcc();
    let config_box = cfg.codec.config_box(&cfg.kind);
    match cfg.kind {
        MediaKind::Video { width, height } => {
            let mut p = Vec::new();
            p.extend_from_slice(&[0u8; 6]); // reserved
            p.extend_from_slice(&1u16.to_be_bytes()); // data reference index
            p.extend_from_slice(&[0u8; 16]); // pre-defined + reserved + pre-defined[3]
            p.extend_from_slice(&width.to_be_bytes());
            p.extend_from_slice(&height.to_be_bytes());
            p.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // horiz resolution 72dpi
            p.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // vert resolution 72dpi
            p.extend_from_slice(&0u32.to_be_bytes()); // reserved
            p.extend_from_slice(&1u16.to_be_bytes()); // frame count
            p.extend_from_slice(&[0u8; 32]); // compressor name
            p.extend_from_slice(&0x0018u16.to_be_bytes()); // depth 24-bit
            p.extend_from_slice(&0xffffu16.to_be_bytes()); // pre-defined (-1)
            p.extend_from_slice(&config_box);
            boxed(fourcc, &p)
        }
        MediaKind::Audio {
            sample_rate,
            channels,
        } => {
            let mut p = Vec::new();
            p.extend_from_slice(&[0u8; 6]); // reserved
            p.extend_from_slice(&1u16.to_be_bytes()); // data reference index
            p.extend_from_slice(&[0u8; 8]); // version/revision/vendor
            p.extend_from_slice(&channels.to_be_bytes());
            p.extend_from_slice(&16u16.to_be_bytes()); // sample size
            p.extend_from_slice(&0u16.to_be_bytes()); // pre-defined
            p.extend_from_slice(&0u16.to_be_bytes()); // reserved
            p.extend_from_slice(&(sample_rate << 16).to_be_bytes()); // 16.16 fixed
            p.extend_from_slice(&config_box);
            boxed(fourcc, &p)
        }
    }
}

// ----------------------------------------------------------------------------
// Codec config boxes
// ----------------------------------------------------------------------------

/// MPEG-4 expandable descriptor length (7 bits/byte, high bit = continuation).
fn descriptor(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut len_bytes = Vec::new();
    let mut len = payload.len();
    loop {
        let mut byte = (len & 0x7f) as u8;
        len >>= 7;
        if !len_bytes.is_empty() {
            byte |= 0x80;
        }
        len_bytes.push(byte);
        if len == 0 {
            break;
        }
    }
    len_bytes.reverse();
    let mut out = Vec::with_capacity(1 + len_bytes.len() + payload.len());
    out.push(tag);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(payload);
    out
}

/// Build an `esds` box wrapping the AAC AudioSpecificConfig.
fn build_esds(asc: &[u8]) -> Vec<u8> {
    let dec_specific = descriptor(0x05, asc); // DecoderSpecificInfo

    let mut dcd = Vec::new();
    dcd.push(0x40); // objectTypeIndication: Audio ISO/IEC 14496-3
    dcd.push(0x15); // streamType=5 (audio) << 2 | upstream=0 | reserved=1
    dcd.extend_from_slice(&[0, 0, 0]); // bufferSizeDB
    dcd.extend_from_slice(&0u32.to_be_bytes()); // maxBitrate
    dcd.extend_from_slice(&0u32.to_be_bytes()); // avgBitrate
    dcd.extend_from_slice(&dec_specific);
    let dec_config = descriptor(0x04, &dcd); // DecoderConfigDescriptor

    let sl = descriptor(0x06, &[0x02]); // SLConfigDescriptor: predefined

    let mut es = Vec::new();
    es.extend_from_slice(&0u16.to_be_bytes()); // ES_ID
    es.push(0x00); // flags
    es.extend_from_slice(&dec_config);
    es.extend_from_slice(&sl);
    let es_descriptor = descriptor(0x03, &es); // ES_Descriptor

    let mut payload = Vec::new();
    payload.extend_from_slice(&0u32.to_be_bytes()); // FullBox version + flags
    payload.extend_from_slice(&es_descriptor);
    boxed(b"esds", &payload)
}

/// Build a `dOps` box from the MKV OpusHead `CodecPrivate`.
///
/// OpusHead layout: "OpusHead"(8) | version(1) | channels(1) | preskip(2 LE) |
/// rate(4 LE) | gain(2 LE) | mapping_family(1) | [channel mapping…]. The dOps
/// payload mirrors this without the magic and with the multi-byte fields big-endian.
fn build_dops(head: &[u8], kind: &MediaKind) -> Vec<u8> {
    let channels_fallback = match kind {
        MediaKind::Audio { channels, .. } => *channels as u8,
        _ => 2,
    };
    // Strip the 8-byte "OpusHead" magic if present.
    let body = if head.len() >= 8 && &head[0..8] == b"OpusHead" {
        &head[8..]
    } else {
        head
    };

    let channels = body.get(1).copied().unwrap_or(channels_fallback);
    let preskip = body
        .get(2..4)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .unwrap_or(3840);
    let rate = body
        .get(4..8)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(48000);
    let gain = body
        .get(8..10)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .unwrap_or(0);
    let mapping_family = body.get(10).copied().unwrap_or(0);

    let mut p = Vec::new();
    p.push(0); // version
    p.push(channels);
    p.extend_from_slice(&preskip.to_be_bytes());
    p.extend_from_slice(&rate.to_be_bytes());
    p.extend_from_slice(&gain.to_be_bytes());
    p.push(mapping_family);
    if mapping_family != 0 {
        // StreamCount, CoupledCount, ChannelMapping[channels] follow in OpusHead.
        if let Some(rest) = body.get(11..) {
            p.extend_from_slice(rest);
        }
    }
    boxed(b"dOps", &p)
}

// ============================================================================
// Media segment (moof + mdat)
// ============================================================================

pub fn build_media_segment(
    track_id: u32,
    sequence_number: u32,
    base_media_decode_time: u64,
    samples: &[Sample],
) -> Vec<u8> {
    // Build once to size the moof, then rebuild with the correct mdat data offset.
    let moof_probe = build_moof(track_id, sequence_number, base_media_decode_time, samples, 0);
    let data_offset = moof_probe.len() as u32 + 8; // + mdat header
    let moof = build_moof(
        track_id,
        sequence_number,
        base_media_decode_time,
        samples,
        data_offset,
    );

    let mdat_payload_size: usize = samples.iter().map(|s| s.data.len()).sum();
    let mut out = Vec::with_capacity(moof.len() + 8 + mdat_payload_size);
    out.extend_from_slice(&moof);
    out.extend_from_slice(&((8 + mdat_payload_size) as u32).to_be_bytes());
    out.extend_from_slice(b"mdat");
    for s in samples {
        out.extend_from_slice(&s.data);
    }
    out
}

fn build_moof(
    track_id: u32,
    sequence_number: u32,
    base_media_decode_time: u64,
    samples: &[Sample],
    data_offset: u32,
) -> Vec<u8> {
    let mut mfhd = Vec::new();
    mfhd.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    mfhd.extend_from_slice(&sequence_number.to_be_bytes());
    let mfhd = boxed(b"mfhd", &mfhd);

    let traf = build_traf(track_id, base_media_decode_time, samples, data_offset);
    boxed_concat(b"moof", &[&mfhd, &traf])
}

fn build_traf(
    track_id: u32,
    base_media_decode_time: u64,
    samples: &[Sample],
    data_offset: u32,
) -> Vec<u8> {
    // tfhd: default-base-is-moof (0x020000)
    let mut tfhd = Vec::new();
    tfhd.extend_from_slice(&0x0002_0000u32.to_be_bytes());
    tfhd.extend_from_slice(&track_id.to_be_bytes());
    let tfhd = boxed(b"tfhd", &tfhd);

    // tfdt: version 1, 64-bit absolute base media decode time
    let mut tfdt = Vec::new();
    tfdt.extend_from_slice(&0x0100_0000u32.to_be_bytes());
    tfdt.extend_from_slice(&base_media_decode_time.to_be_bytes());
    let tfdt = boxed(b"tfdt", &tfdt);

    let trun = build_trun(samples, data_offset);
    boxed_concat(b"traf", &[&tfhd, &tfdt, &trun])
}

fn build_trun(samples: &[Sample], data_offset: u32) -> Vec<u8> {
    // data-offset + sample-duration + sample-size + sample-flags + sample-cts
    let flags: u32 = 0x000001 | 0x000100 | 0x000200 | 0x000400 | 0x000800;
    let mut p = Vec::new();
    p.extend_from_slice(&(0x0100_0000 | flags).to_be_bytes()); // version 1 (signed cts)
    p.extend_from_slice(&(samples.len() as u32).to_be_bytes());
    p.extend_from_slice(&(data_offset as i32).to_be_bytes());
    for s in samples {
        p.extend_from_slice(&s.duration.to_be_bytes());
        p.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
        let sample_flags: u32 = if s.is_sync { 0x0200_0000 } else { 0x0101_0000 };
        p.extend_from_slice(&sample_flags.to_be_bytes());
        p.extend_from_slice(&s.cts_offset.to_be_bytes());
    }
    boxed(b"trun", &p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_box(data: &[u8], typ: &[u8; 4]) -> Option<usize> {
        data.windows(4)
            .position(|w| w == typ)
            .and_then(|pos| pos.checked_sub(4))
    }

    fn avc_cfg() -> TrackConfig {
        TrackConfig {
            track_id: 1,
            timescale: 1000,
            duration: 0,
            language: "eng".to_string(),
            kind: MediaKind::Video {
                width: 1280,
                height: 544,
            },
            codec: CodecConfig::Avc(vec![1, 0x64, 0, 0x28, 0xff, 0xe1, 0, 4, 0x67, 1, 2, 3]),
        }
    }

    #[test]
    fn init_segment_has_ftyp_then_moov() {
        let init = build_init_segment(&avc_cfg());
        assert_eq!(&init[4..8], b"ftyp");
        let ftyp_size = u32::from_be_bytes(init[0..4].try_into().unwrap()) as usize;
        assert_eq!(&init[ftyp_size + 4..ftyp_size + 8], b"moov");
        assert!(find_box(&init, b"mvex").is_some());
        assert!(find_box(&init, b"trex").is_some());
        assert!(find_box(&init, b"avcC").is_some());
    }

    #[test]
    fn audio_init_has_soun_smhd_esds() {
        let cfg = TrackConfig {
            track_id: 2,
            timescale: 48000,
            duration: 0,
            language: "eng".to_string(),
            kind: MediaKind::Audio {
                sample_rate: 48000,
                channels: 2,
            },
            codec: CodecConfig::Aac(vec![0x11, 0x90]),
        };
        let init = build_init_segment(&cfg);
        assert!(find_box(&init, b"soun").is_some());
        assert!(find_box(&init, b"smhd").is_some());
        assert!(find_box(&init, b"mp4a").is_some());
        assert!(find_box(&init, b"esds").is_some());
    }

    #[test]
    fn media_segment_moof_then_mdat_and_offset_points_into_mdat() {
        let samples = vec![
            Sample {
                data: vec![0, 0, 0, 4, 1, 2, 3, 4],
                duration: 42,
                cts_offset: 0,
                is_sync: true,
            },
            Sample {
                data: vec![0, 0, 0, 2, 9, 9],
                duration: 42,
                cts_offset: 42,
                is_sync: false,
            },
        ];
        let seg = build_media_segment(1, 1, 9000, &samples);
        assert_eq!(&seg[4..8], b"moof");
        let moof_size = u32::from_be_bytes(seg[0..4].try_into().unwrap()) as usize;
        assert_eq!(&seg[moof_size + 4..moof_size + 8], b"mdat");

        // data_offset in trun must equal moof_size + 8 (start of mdat payload).
        let trun = find_box(&seg, b"trun").unwrap();
        let data_offset = i32::from_be_bytes(seg[trun + 16..trun + 20].try_into().unwrap());
        assert_eq!(data_offset as usize, moof_size + 8);

        // tfdt carries the absolute base decode time we passed.
        let tfdt = find_box(&seg, b"tfdt").unwrap();
        let base = u64::from_be_bytes(seg[tfdt + 12..tfdt + 20].try_into().unwrap());
        assert_eq!(base, 9000);
    }
}
