//! Track model: parses Matroska `TrackEntry` elements into [`TrackData`] and maps
//! Matroska codec IDs to the MSE codec strings used with
//! `MediaSource.isTypeSupported` / `SourceBuffer`.

use matroska_ebml::ebml::{EbmlIterator, EbmlPayload, EbmlSource};
use matroska_ebml::matroska_data::{
    ID_AUDIO, ID_BITDEPTH, ID_CHANNELS, ID_CODECDELAY, ID_CODECID, ID_CODECNAME, ID_CODECPRIVATE,
    ID_CONTENTCOMPALGO, ID_CONTENTCOMPRESSION, ID_CONTENTCOMPSETTINGS, ID_CONTENTENCODING,
    ID_CONTENTENCODINGS, ID_DEFAULTDURATION, ID_DISPLAYHEIGHT, ID_DISPLAYWIDTH, ID_FLAGDEFAULT,
    ID_FLAGFORCED, ID_LANGUAGE, ID_LANGUAGEBCP47, ID_NAME, ID_PIXELHEIGHT, ID_PIXELWIDTH,
    ID_SAMPLINGFREQUENCY, ID_SEEKPREROLL, ID_TRACKENTRY, ID_TRACKNUMBER, ID_TRACKTYPE, ID_TRACKUID,
    ID_VIDEO,
};

/// A track's `ContentCompression` (`\...\ContentEncodings\ContentEncoding\ContentCompression`).
/// Matroska subtitle tracks are commonly zlib-compressed per block by mkvmerge; block payloads
/// must be decompressed before use. See [`decode_block_frame`].
#[derive(Debug, Clone)]
pub struct Compression {
    /// `ContentCompAlgo`: 0 = zlib, 1 = bzlib, 2 = lzo1x, 3 = header-strip.
    pub algo: u64,
    /// `ContentCompSettings`: for header-strip (algo 3), the bytes stripped from each frame.
    pub settings: Option<Vec<u8>>,
}

/// Decompress one block frame per a track's `ContentCompression`, or return it unchanged when
/// the track isn't compressed. Returns `None` for an unsupported algorithm or corrupt data
/// (caller drops the frame). Handles the two algorithms seen in the wild for subtitles: zlib
/// (0) and header-strip (3); bzlib/lzo1x (1/2) are unsupported.
pub fn decode_block_frame(comp: Option<&Compression>, frame: &[u8]) -> Option<Vec<u8>> {
    match comp {
        None => Some(frame.to_vec()),
        Some(c) => match c.algo {
            0 => miniz_oxide::inflate::decompress_to_vec_zlib(frame).ok(),
            3 => {
                let mut out = c.settings.clone().unwrap_or_default();
                out.extend_from_slice(frame);
                Some(out)
            }
            _ => None,
        },
    }
}

/// Matroska `TrackType` values (`\Segment\Tracks\TrackEntry\TrackType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackType {
    Video,
    Audio,
    Complex,
    Logo,
    Subtitle,
    Buttons,
    Control,
    Metadata,
}

impl TrackType {
    pub fn from_u64(value: u64) -> Option<Self> {
        Some(match value {
            1 => TrackType::Video,
            2 => TrackType::Audio,
            3 => TrackType::Complex,
            16 => TrackType::Logo,
            17 => TrackType::Subtitle,
            18 => TrackType::Buttons,
            32 => TrackType::Control,
            33 => TrackType::Metadata,
            _ => return None,
        })
    }

    /// Stable lowercase tag used in the JSON handed to JS.
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackType::Video => "video",
            TrackType::Audio => "audio",
            TrackType::Subtitle => "subtitle",
            TrackType::Complex => "complex",
            TrackType::Logo => "logo",
            TrackType::Buttons => "buttons",
            TrackType::Control => "control",
            TrackType::Metadata => "metadata",
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct TrackData {
    pub track_number: Option<u64>,
    pub track_uid: Option<u64>,
    pub track_type: Option<TrackType>,

    pub codec_id: Option<String>,
    pub codec_private: Option<Vec<u8>>,
    pub codec_name: Option<String>,
    /// Human-readable track title (`Name` element), e.g. "Signs & Songs", "English (SDH)".
    pub name: Option<String>,

    /// Nanoseconds per frame (`DefaultDuration`). Needed for sample durations.
    pub default_duration: Option<u64>,
    pub language: Option<String>,
    pub language_bcp47: Option<String>,
    pub flag_default: bool,
    pub flag_forced: bool,

    // Video
    pub pixel_width: Option<u64>,
    pub pixel_height: Option<u64>,
    pub display_width: Option<u64>,
    pub display_height: Option<u64>,

    // Audio
    pub sampling_frequency: Option<f64>,
    pub channels: Option<u64>,
    pub bit_depth: Option<u64>,
    /// `CodecDelay` / `SeekPreroll` in nanoseconds (Opus).
    pub codec_delay: Option<u64>,
    pub seek_preroll: Option<u64>,

    /// Per-block content compression, if any (subtitle tracks are often zlib-compressed).
    pub compression: Option<Compression>,
}

impl TrackData {
    /// BCP-47 language, preferring the explicit `LanguageBCP47` element, then the
    /// legacy ISO-639 `Language` (Matroska default is `"eng"`).
    pub fn language_tag(&self) -> &str {
        self.language_bcp47
            .as_deref()
            .or(self.language.as_deref())
            .unwrap_or("eng")
    }

    /// The `video/mp4` / `audio/mp4` MIME string for `isTypeSupported`, or `None`
    /// for tracks that are not muxed into MP4 (subtitles, unknown codecs).
    pub fn mime_type(&self) -> Option<String> {
        let codec = self.codec_string()?;
        let container = match self.track_type? {
            TrackType::Video => "video/mp4",
            TrackType::Audio => "audio/mp4",
            _ => return None,
        };
        Some(format!("{}; codecs=\"{}\"", container, codec))
    }

    /// RFC 6381 codec parameter (e.g. `avc1.640028`, `mp4a.40.2`), or `None` if the
    /// codec is not supported by the muxer.
    pub fn codec_string(&self) -> Option<String> {
        let codec_id = self.codec_id.as_deref()?;
        let cp = self.codec_private.as_deref();
        match codec_id {
            "V_MPEG4/ISO/AVC" => Some(avc_codec_string(cp?)),
            "V_MPEGH/ISO/HEVC" => Some(hevc_codec_string(cp?)),
            "V_VP9" => Some(self.vp9_codec_string()),
            "V_AV1" => Some(av1_codec_string(cp)),
            "A_AAC" => Some(aac_codec_string(cp)),
            "A_OPUS" => Some("opus".to_string()),
            "A_AC3" => Some("ac-3".to_string()),
            "A_EAC3" => Some("ec-3".to_string()),
            "A_FLAC" => Some("flac".to_string()),
            // MPEG-1/2 Audio Layer III, muxed as mp4a with object-type 0x6B.
            "A_MPEG/L3" => Some("mp4a.6B".to_string()),
            _ => None,
        }
    }

    /// fMP4 sample-entry FourCC for this codec (`avc1`, `hvc1`, `mp4a`, ...).
    pub fn sample_entry(&self) -> Option<&'static str> {
        Some(match self.codec_id.as_deref()? {
            "V_MPEG4/ISO/AVC" => "avc1",
            "V_MPEGH/ISO/HEVC" => "hvc1",
            "V_VP9" => "vp09",
            "V_AV1" => "av01",
            "A_AAC" => "mp4a",
            "A_OPUS" => "Opus",
            "A_AC3" => "ac-3",
            "A_EAC3" => "ec-3",
            "A_FLAC" => "fLaC",
            "A_MPEG/L3" => "mp4a",
            _ => return None,
        })
    }

    /// `vp09.PP.LL.DD` for VP9. Profile, level, and bit depth come from the VP9
    /// codec feature metadata in CodecPrivate when present; otherwise Profile 0 /
    /// 8-bit is assumed and the level is computed from the frame size and rate.
    /// Unlike a fixed string, this lets `isTypeSupported` judge the real stream
    /// (e.g. a 10-bit Profile 2 clip reports `vp09.02.LL.10`, not `vp09.00.10.08`).
    fn vp9_codec_string(&self) -> String {
        let mut profile: u8 = 0;
        let mut level: Option<u8> = None;
        let mut bit_depth: u8 = 8;

        // WebM VP9 CodecPrivate (when present) is a sequence of [id:1][len:1][value:len]
        // fields: id 1 = Profile, 2 = Level, 3 = Bit Depth.
        if let Some(cp) = self.codec_private.as_deref() {
            let mut i = 0;
            while i + 2 <= cp.len() {
                let id = cp[i];
                let len = cp[i + 1] as usize;
                if let Some(v) = cp.get(i + 2..i + 2 + len).and_then(<[u8]>::first).copied() {
                    match id {
                        1 => profile = v,
                        2 => level = Some(v),
                        3 => bit_depth = v,
                        _ => {}
                    }
                }
                i += 2 + len;
            }
        }

        let level = level.unwrap_or_else(|| self.vp9_level());
        format!("vp09.{:02}.{:02}.{:02}", profile, level, bit_depth)
    }

    /// Lowest VP9 level (per the spec's level table) whose limits cover this track's
    /// luma picture size and sample rate. Frame rate comes from `DefaultDuration`
    /// (defaulting to 30 fps when absent); unknown dimensions yield level 1.0.
    fn vp9_level(&self) -> u8 {
        let picture_size = self
            .pixel_width
            .unwrap_or(0)
            .saturating_mul(self.pixel_height.unwrap_or(0));
        let fps = self
            .default_duration
            .filter(|&d| d > 0)
            .map(|d| 1_000_000_000.0 / d as f64)
            .unwrap_or(30.0);
        let sample_rate = picture_size as f64 * fps;

        // (level, max luma sample rate, max luma picture size) — VP9 spec Table.
        const LEVELS: &[(u8, f64, u64)] = &[
            (10, 829_440.0, 36_864),
            (11, 2_764_800.0, 73_728),
            (20, 4_608_000.0, 122_880),
            (21, 9_216_000.0, 245_760),
            (30, 20_736_000.0, 552_960),
            (31, 36_864_000.0, 983_040),
            (40, 83_558_400.0, 2_228_224),
            (41, 160_432_128.0, 2_228_224),
            (50, 311_951_360.0, 8_912_896),
            (51, 588_251_136.0, 8_912_896),
            (52, 1_176_502_272.0, 8_912_896),
            (60, 1_176_502_272.0, 35_651_584),
            (61, 2_353_004_544.0, 35_651_584),
            (62, 4_706_009_088.0, 35_651_584),
        ];
        for &(level, max_rate, max_size) in LEVELS {
            if picture_size <= max_size && sample_rate <= max_rate {
                return level;
            }
        }
        62
    }
}

/// `avc1.PPCCLL` from the AVCDecoderConfigurationRecord (bytes 1..=3).
fn avc_codec_string(avcc: &[u8]) -> String {
    if avcc.len() >= 4 {
        format!("avc1.{:02X}{:02X}{:02X}", avcc[1], avcc[2], avcc[3])
    } else {
        "avc1.640028".to_string()
    }
}

/// `mp4a.40.N` where N is the AAC object type from the AudioSpecificConfig
/// (first 5 bits). Defaults to LC (`.2`) when CodecPrivate is missing.
fn aac_codec_string(asc: Option<&[u8]>) -> String {
    let object_type = asc
        .and_then(|b| b.first())
        .map(|b| b >> 3)
        .filter(|&t| t != 0)
        .unwrap_or(2);
    format!("mp4a.40.{}", object_type)
}

/// Best-effort `av01.P.LLT.DD` from the av1C record. Falls back to a Main-profile
/// 8-bit string when the record is unavailable.
fn av1_codec_string(av1c: Option<&[u8]>) -> String {
    if let Some(c) = av1c {
        if c.len() >= 3 {
            let seq_profile = (c[1] >> 5) & 0x07;
            let seq_level = c[1] & 0x1f;
            let tier = if c[2] & 0x80 != 0 { 'H' } else { 'M' };
            let high_bitdepth = c[2] & 0x40 != 0;
            let twelve_bit = c[2] & 0x20 != 0;
            let bit_depth = if twelve_bit {
                12
            } else if high_bitdepth {
                10
            } else {
                8
            };
            return format!(
                "av01.{}.{:02}{}.{:02}",
                seq_profile, seq_level, tier, bit_depth
            );
        }
    }
    "av01.0.04M.08".to_string()
}

/// `hvc1.…` from the HEVCDecoderConfigurationRecord (the MKV CodecPrivate).
/// Format per ISO 14496-15: `hvc1.PS.compat.TLvl.constraints`.
fn hevc_codec_string(hvcc: &[u8]) -> String {
    // Layout: [0]=version, [1]=profile_space(2)|tier(1)|profile_idc(5),
    // [2..6]=compatibility_flags, [6..12]=constraint_flags, [12]=level_idc.
    if hvcc.len() < 13 {
        return "hvc1.1.6.L93.B0".to_string();
    }
    let profile_space = (hvcc[1] >> 6) & 0x03;
    let tier_flag = (hvcc[1] >> 5) & 0x01;
    let profile_idc = hvcc[1] & 0x1f;

    let space_prefix = match profile_space {
        0 => "",
        1 => "A",
        2 => "B",
        _ => "C",
    };

    // Compatibility flags: 32-bit value, emitted as reversed-bit hex (per spec the
    // string carries the flags with bit order reversed).
    let compat = u32::from_be_bytes([hvcc[2], hvcc[3], hvcc[4], hvcc[5]]);
    let compat_rev = compat.reverse_bits();

    let tier_char = if tier_flag == 1 { 'H' } else { 'L' };
    let level = hvcc[12];

    // Constraint bytes: trailing zero bytes are omitted; emit the significant ones.
    let constraints = &hvcc[6..12];
    let last_nonzero = constraints.iter().rposition(|&b| b != 0);
    let mut constraint_str = String::new();
    if let Some(end) = last_nonzero {
        for b in &constraints[..=end] {
            constraint_str.push_str(&format!(".{:02X}", b));
        }
    }

    format!(
        "hvc1.{}{}.{:X}.{}{}{}",
        space_prefix, profile_idc, compat_rev, tier_char, level, constraint_str
    )
}

/// Parse all `TrackEntry` elements from a `Tracks` master iterator.
pub async fn parse_tracks<S>(mut tracks: EbmlIterator<S>) -> Vec<TrackData>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut result = Vec::new();
    while let Some(entry) = tracks.next().await {
        if entry.id == ID_TRACKENTRY {
            if let EbmlPayload::Master(payload) = entry.payload {
                result.push(parse_track_entry(payload).await);
            }
        }
    }
    result
}

async fn parse_track_entry<S>(mut payload: EbmlIterator<S>) -> TrackData
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut track = TrackData::default();

    while let Some(field) = payload.next().await {
        match field.payload {
            EbmlPayload::UnsignedInt(v) => match field.id {
                ID_TRACKNUMBER => track.track_number = Some(v),
                ID_TRACKUID => track.track_uid = Some(v),
                ID_TRACKTYPE => track.track_type = TrackType::from_u64(v),
                ID_DEFAULTDURATION => track.default_duration = Some(v),
                ID_FLAGDEFAULT => track.flag_default = v != 0,
                ID_FLAGFORCED => track.flag_forced = v != 0,
                ID_CODECDELAY => track.codec_delay = Some(v),
                ID_SEEKPREROLL => track.seek_preroll = Some(v),
                _ => {}
            },
            EbmlPayload::String(s) => match field.id {
                ID_CODECID => track.codec_id = Some(s),
                ID_CODECNAME => track.codec_name = Some(s),
                ID_NAME => track.name = Some(s),
                ID_LANGUAGE => track.language = Some(s),
                ID_LANGUAGEBCP47 => track.language_bcp47 = Some(s),
                _ => {}
            },
            EbmlPayload::Binary((start, end)) if field.id == ID_CODECPRIVATE => {
                track.codec_private = Some(payload.read_range(start, end).await.into_vec());
            }
            EbmlPayload::Master(sub) if field.id == ID_VIDEO => {
                parse_video(sub, &mut track).await;
            }
            EbmlPayload::Master(sub) if field.id == ID_AUDIO => {
                parse_audio(sub, &mut track).await;
            }
            EbmlPayload::Master(sub) if field.id == ID_CONTENTENCODINGS => {
                track.compression = parse_content_encodings(sub).await;
            }
            _ => {}
        }
    }

    track
}

/// Parse `ContentEncodings` for the first `ContentCompression` (block-scope compression).
/// Encryption and multi-encoding stacks aren't supported — we take the first compression.
async fn parse_content_encodings<S>(mut encodings: EbmlIterator<S>) -> Option<Compression>
where
    S: EbmlSource + PartialEq + Clone,
{
    while let Some(enc) = encodings.next().await {
        let EbmlPayload::Master(mut encoding) = enc.payload else { continue };
        if enc.id != ID_CONTENTENCODING {
            continue;
        }
        while let Some(field) = encoding.next().await {
            let EbmlPayload::Master(mut comp) = field.payload else { continue };
            if field.id != ID_CONTENTCOMPRESSION {
                continue;
            }
            let mut algo = 0u64; // Matroska default ContentCompAlgo is 0 (zlib)
            let mut settings = None;
            while let Some(cf) = comp.next().await {
                match cf.payload {
                    EbmlPayload::UnsignedInt(v) if cf.id == ID_CONTENTCOMPALGO => algo = v,
                    EbmlPayload::Binary((s, e)) if cf.id == ID_CONTENTCOMPSETTINGS => {
                        settings = Some(comp.read_range(s, e).await.into_vec());
                    }
                    _ => {}
                }
            }
            return Some(Compression { algo, settings });
        }
    }
    None
}

async fn parse_video<S>(mut video: EbmlIterator<S>, track: &mut TrackData)
where
    S: EbmlSource + PartialEq + Clone,
{
    while let Some(field) = video.next().await {
        if let EbmlPayload::UnsignedInt(v) = field.payload {
            match field.id {
                ID_PIXELWIDTH => track.pixel_width = Some(v),
                ID_PIXELHEIGHT => track.pixel_height = Some(v),
                ID_DISPLAYWIDTH => track.display_width = Some(v),
                ID_DISPLAYHEIGHT => track.display_height = Some(v),
                _ => {}
            }
        }
    }
}

async fn parse_audio<S>(mut audio: EbmlIterator<S>, track: &mut TrackData)
where
    S: EbmlSource + PartialEq + Clone,
{
    while let Some(field) = audio.next().await {
        match field.payload {
            EbmlPayload::UnsignedInt(v) => match field.id {
                ID_CHANNELS => track.channels = Some(v),
                ID_BITDEPTH => track.bit_depth = Some(v),
                _ => {}
            },
            EbmlPayload::Float(v) if field.id == ID_SAMPLINGFREQUENCY => {
                track.sampling_frequency = Some(v);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_block_frame, Compression};

    #[test]
    fn no_compression_returns_frame_unchanged() {
        let frame = b"Dialogue: raw text";
        assert_eq!(decode_block_frame(None, frame).unwrap(), frame);
    }

    #[test]
    fn zlib_frame_is_inflated() {
        let original = b"\x16\x00\x03\x01\x02\x03\x80\x00\x00"; // a fake PGS-ish payload
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(original, 6);
        assert_ne!(&compressed[..], &original[..], "precondition: actually compressed");
        let comp = Compression { algo: 0, settings: None };
        assert_eq!(decode_block_frame(Some(&comp), &compressed).unwrap(), original);
    }

    #[test]
    fn corrupt_zlib_frame_is_dropped() {
        let comp = Compression { algo: 0, settings: None };
        assert_eq!(decode_block_frame(Some(&comp), b"not zlib data"), None);
    }

    #[test]
    fn header_strip_prepends_settings() {
        let comp = Compression { algo: 3, settings: Some(vec![0xAA, 0xBB]) };
        assert_eq!(decode_block_frame(Some(&comp), b"\x01\x02").unwrap(), vec![0xAA, 0xBB, 0x01, 0x02]);
    }

    #[test]
    fn unsupported_algo_is_dropped() {
        let comp = Compression { algo: 1, settings: None }; // bzlib — unsupported
        assert_eq!(decode_block_frame(Some(&comp), b"x"), None);
    }
}
