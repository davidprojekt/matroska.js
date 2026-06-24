//! Track model: parses Matroska `TrackEntry` elements into [`TrackData`] and maps
//! Matroska codec IDs to the MSE codec strings used with
//! `MediaSource.isTypeSupported` / `SourceBuffer`.

use crate::ebml::{EbmlIterator, EbmlPayload, EbmlSource};
use crate::matroska_data::{
    ID_AUDIO, ID_BITDEPTH, ID_CHANNELS, ID_CODECDELAY, ID_CODECID, ID_CODECNAME, ID_CODECPRIVATE,
    ID_DEFAULTDURATION, ID_DISPLAYHEIGHT, ID_DISPLAYWIDTH, ID_FLAGDEFAULT, ID_FLAGFORCED,
    ID_LANGUAGE, ID_LANGUAGEBCP47, ID_PIXELHEIGHT, ID_PIXELWIDTH, ID_SAMPLINGFREQUENCY,
    ID_SEEKPREROLL, ID_TRACKENTRY, ID_TRACKNUMBER, ID_TRACKTYPE, ID_TRACKUID, ID_VIDEO,
};

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
            "V_VP9" => Some("vp09.00.10.08".to_string()),
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
            _ => {}
        }
    }

    track
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
