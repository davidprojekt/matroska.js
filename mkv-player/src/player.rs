//! Orchestration: parse an MKV Segment, expose tracks + their MSE codec strings,
//! and remux requested time ranges of a track into fragmented-MP4 segments.
//!
//! [`Demuxer`] is generic over [`EbmlSource`] so it can be exercised natively with
//! `FsSource`; [`MatroskaPlayer`] is the `wasm-bindgen` facade over
//! `Demuxer<FetchSource>`.

use ebml_wasm::ebml::{Ebml, EbmlIterator, EbmlPayload, EbmlSource};
use crate::fmp4::{build_init_segment, build_media_segment, CodecConfig, MediaKind, TrackConfig};
use ebml_wasm::mem_source::MemSource;
use crate::index::{cue_cluster_for_time, parse_cues, parse_seek_head, CuePoint};
use ebml_wasm::matroska_data::{
    ID_ATTACHEDFILE, ID_ATTACHMENTS, ID_BLOCK, ID_BLOCKDURATION, ID_BLOCKGROUP, ID_CHAPLANGUAGE,
    ID_CHAPLANGUAGEBCP47, ID_CHAPSTRING, ID_CHAPTERATOM, ID_CHAPTERDISPLAY, ID_CHAPTERFLAGHIDDEN,
    ID_CHAPTERS, ID_CHAPTERTIMEEND, ID_CHAPTERTIMESTART, ID_CHAPTERUID, ID_CLUSTER, ID_CUES,
    ID_DURATION, ID_EDITIONENTRY, ID_EDITIONFLAGDEFAULT, ID_EDITIONFLAGHIDDEN, ID_FILEDATA,
    ID_FILEMEDIATYPE, ID_FILENAME, ID_FILEUID, ID_INFO, ID_REFERENCEBLOCK, ID_SEEKHEAD, ID_SEGMENT,
    ID_SIMPLEBLOCK, ID_TIMESTAMP, ID_TIMESTAMPSCALE, ID_TRACKS,
};
use crate::remux::{
    audio_samples, cues_to_webvtt, parse_block, video_samples, BlockFrames, SubtitleCue, TimedFrame,
};
use crate::track::{parse_tracks, TrackData, TrackType};

const DEFAULT_TIMESTAMP_SCALE_NS: u64 = 1_000_000;
/// EBML IDs that `SeekHead` may point at and we care about.
const SEEK_ID_CUES: u64 = ID_CUES;
const SEEK_ID_TRACKS: u64 = ID_TRACKS;
const SEEK_ID_INFO: u64 = ID_INFO;
const SEEK_ID_ATTACHMENTS: u64 = ID_ATTACHMENTS;
const SEEK_ID_CHAPTERS: u64 = ID_CHAPTERS;

/// One localized chapter title.
#[derive(Debug, Clone)]
pub struct ChapterDisplay {
    pub text: String,
    /// Legacy ISO-639 `ChapLanguage` (Matroska default `"eng"`).
    pub language: String,
    /// BCP-47 `ChapLanguageBCP47`, when present.
    pub language_bcp47: Option<String>,
}

/// One `ChapterAtom`: a start (and optional end) time, plus its localized titles. JS
/// picks the title matching the selected audio language (with fallback).
#[derive(Debug, Clone)]
pub struct ChapterEntry {
    pub uid: u64,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub displays: Vec<ChapterDisplay>,
}

/// One `AttachedFile`: its metadata plus the **absolute** byte range of its `FileData`.
/// The data itself is never read here — only its range — so fonts can be fetched
/// out-of-band by JS (a separate connection) without pulling them through the demuxer.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub uid: u64,
    /// Absolute offset of the first `FileData` byte.
    pub data_start: u64,
    /// Absolute offset of the last `FileData` byte (inclusive).
    pub data_end: u64,
}

/// Whether a `FileMediaType` (or, as a fallback, a filename) denotes a font we should
/// hand to libass. Muxers are inconsistent — some use `font/*`, some the legacy
/// `application/x-truetype-font`, some mislabel fonts as `application/octet-stream`.
fn is_font_attachment(mime: &str, name: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    if m.starts_with("font/")
        || m.contains("truetype")
        || m.contains("opentype")
        || m.contains("font-sfnt")
        || m.contains("vnd.ms-opentype")
        || m == "application/x-font-ttf"
        || m == "application/x-font-otf"
    {
        return true;
    }
    // Fallback for octet-stream / empty mimes: trust the extension.
    let n = name.to_ascii_lowercase();
    n.ends_with(".ttf") || n.ends_with(".otf") || n.ends_with(".ttc") || n.ends_with(".otc")
}

pub struct Demuxer<S>
where
    S: EbmlSource + PartialEq + Clone,
{
    ebml: Ebml<S>,
    /// Offset where the Segment master element's *content* starts. Base for every
    /// SeekHead/Cues position.
    segment_data_start: u64,
    segment_end: u64,
    timestamp_scale_ns: u64,
    /// Total duration in TimestampScale ticks (from Info\Duration).
    duration_ticks: f64,
    tracks: Vec<TrackData>,
    cues: Vec<CuePoint>,
    first_cluster_offset: Option<u64>,
    attachments: Vec<Attachment>,
    chapters: Vec<ChapterEntry>,
}

impl<S> Demuxer<S>
where
    S: EbmlSource + PartialEq + Clone,
{
    pub async fn open(ebml: Ebml<S>) -> Self {
        let mut demux = Demuxer {
            ebml,
            segment_data_start: 0,
            segment_end: u64::MAX,
            timestamp_scale_ns: DEFAULT_TIMESTAMP_SCALE_NS,
            duration_ticks: 0.0,
            tracks: Vec::new(),
            cues: Vec::new(),
            first_cluster_offset: None,
            attachments: Vec::new(),
            chapters: Vec::new(),
        };
        demux.parse().await;
        demux
    }

    fn iter_at(&self, offset: u64, end: Option<u64>) -> EbmlIterator<S> {
        match end {
            Some(end) => EbmlIterator::new(offset, end, self.ebml.clone()),
            None => EbmlIterator::new_endless(offset, self.ebml.clone()),
        }
    }

    /// Read `[content_start, content_end)` in a single request and return an iterator
    /// that parses it entirely from memory — avoiding a network round-trip per field.
    async fn buffer_range(&self, content_start: u64, content_end: u64) -> EbmlIterator<MemSource> {
        let end = content_end.min(self.segment_end);
        let bytes = if end > content_start {
            self.ebml.source.read_range(content_start, end - 1).await
        } else {
            Vec::new()
        };
        let mem = Ebml::new(MemSource::new(bytes, content_start), self.ebml.id_map.clone());
        EbmlIterator::new(content_start, end, mem)
    }

    /// Read the element at absolute offset `abs` and, if it is a master, return an
    /// iterator over its content **without** buffering it. Used for Attachments, whose
    /// `FileData` payloads can be many MB — buffering would download every font.
    async fn master_at(&self, abs: u64) -> Option<EbmlIterator<S>> {
        let mut it = self.iter_at(abs, Some(self.segment_end));
        let el = it.next().await?;
        if let EbmlPayload::Master(m) = el.payload {
            Some(m)
        } else {
            None
        }
    }

    /// Read the element at absolute offset `abs` and, if it is a master, buffer its
    /// whole content into memory for field-by-field parsing.
    async fn buffered_master_at(&self, abs: u64) -> Option<EbmlIterator<MemSource>> {
        let mut it = self.iter_at(abs, Some(self.segment_end));
        let el = it.next().await?;
        if let EbmlPayload::Master(m) = el.payload {
            Some(self.buffer_range(m.current, m.end.unwrap_or(self.segment_end)).await)
        } else {
            None
        }
    }

    async fn parse(&mut self) {
        // Top level: EBML header, then Segment.
        let mut top = self.iter_at(0, None);
        let mut segment_payload = None;
        while let Some(el) = top.next().await {
            if el.id == ID_SEGMENT {
                if let EbmlPayload::Master(seg) = el.payload {
                    self.segment_data_start = seg.current;
                    self.segment_end = seg.end.unwrap_or(u64::MAX);
                    segment_payload = Some(seg);
                }
                break;
            }
        }
        let Some(mut segment) = segment_payload else {
            return;
        };

        let mut seek_entries = Vec::new();
        let mut have_tracks = false;
        let mut have_cues = false;
        let mut have_attachments = false;
        let mut have_chapters = false;

        // Forward scan of Segment children up to the first Cluster.
        while let Some(el) = segment.next().await {
            match el.id {
                ID_SEEKHEAD => {
                    if let EbmlPayload::Master(sh) = el.payload {
                        seek_entries = parse_seek_head(sh).await;
                    }
                }
                ID_INFO => {
                    if let EbmlPayload::Master(info) = el.payload {
                        self.parse_info(info).await;
                    }
                }
                ID_TRACKS => {
                    if let EbmlPayload::Master(tr) = el.payload {
                        let it = self.buffer_range(tr.current, tr.end.unwrap_or(self.segment_end)).await;
                        self.tracks = parse_tracks(it).await;
                        have_tracks = true;
                    }
                }
                ID_CUES => {
                    if let EbmlPayload::Master(cues) = el.payload {
                        let it = self.buffer_range(cues.current, cues.end.unwrap_or(self.segment_end)).await;
                        self.cues = parse_cues(it).await;
                        have_cues = true;
                    }
                }
                ID_ATTACHMENTS => {
                    if let EbmlPayload::Master(att) = el.payload {
                        self.attachments = parse_attachments(att).await;
                        have_attachments = true;
                    }
                }
                ID_CHAPTERS => {
                    if let EbmlPayload::Master(ch) = el.payload {
                        let it = self.buffer_range(ch.current, ch.end.unwrap_or(self.segment_end)).await;
                        self.chapters = parse_chapters(it).await;
                        have_chapters = true;
                    }
                }
                ID_CLUSTER => {
                    self.first_cluster_offset = Some(el.offset);
                    break;
                }
                _ => {}
            }
        }

        // Anything that lives after the clusters (commonly Cues, sometimes Tracks)
        // is reachable through the SeekHead.
        for entry in &seek_entries {
            let abs = self.segment_data_start + entry.position;
            match entry.seek_id {
                SEEK_ID_CUES if !have_cues => {
                    if let Some(it) = self.buffered_master_at(abs).await {
                        self.cues = parse_cues(it).await;
                        have_cues = true;
                    }
                }
                SEEK_ID_TRACKS if !have_tracks => {
                    if let Some(it) = self.buffered_master_at(abs).await {
                        self.tracks = parse_tracks(it).await;
                        have_tracks = true;
                    }
                }
                SEEK_ID_INFO if self.duration_ticks == 0.0 => {
                    if let Some(it) = self.buffered_master_at(abs).await {
                        self.parse_info(it).await;
                    }
                }
                SEEK_ID_ATTACHMENTS if !have_attachments => {
                    if let Some(it) = self.master_at(abs).await {
                        self.attachments = parse_attachments(it).await;
                        have_attachments = true;
                    }
                }
                SEEK_ID_CHAPTERS if !have_chapters => {
                    if let Some(it) = self.buffered_master_at(abs).await {
                        self.chapters = parse_chapters(it).await;
                        have_chapters = true;
                    }
                }
                _ => {}
            }
        }
    }

    async fn parse_info<M: EbmlSource + PartialEq + Clone>(&mut self, mut info: EbmlIterator<M>) {
        while let Some(field) = info.next().await {
            match field.payload {
                EbmlPayload::UnsignedInt(v) if field.id == ID_TIMESTAMPSCALE => {
                    self.timestamp_scale_ns = v;
                }
                EbmlPayload::Float(v) if field.id == ID_DURATION => self.duration_ticks = v,
                _ => {}
            }
        }
    }

    fn track(&self, track_number: u64) -> Option<&TrackData> {
        self.tracks.iter().find(|t| t.track_number == Some(track_number))
    }

    /// MP4 media timescale for a track: the 1/TimestampScale rate for video (so MKV
    /// ticks map 1:1), and the sample rate for audio.
    fn timescale_for(&self, track: &TrackData) -> u32 {
        match track.track_type {
            Some(TrackType::Audio) => track.sampling_frequency.unwrap_or(48000.0) as u32,
            _ => (1_000_000_000 / self.timestamp_scale_ns.max(1)) as u32,
        }
    }

    fn codec_config(&self, track: &TrackData) -> Option<CodecConfig> {
        let cp = track.codec_private.clone();
        Some(match track.codec_id.as_deref()? {
            "V_MPEG4/ISO/AVC" => CodecConfig::Avc(cp?),
            "V_MPEGH/ISO/HEVC" => CodecConfig::Hevc(cp?),
            "V_AV1" => CodecConfig::Av1(cp?),
            "V_VP9" => CodecConfig::Vp9(cp),
            "A_AAC" => CodecConfig::Aac(cp?),
            "A_OPUS" => CodecConfig::Opus(cp?),
            "A_AC3" | "A_EAC3" => CodecConfig::Ac3(cp.unwrap_or_default()),
            "A_FLAC" => CodecConfig::Flac(cp?),
            "A_MPEG/L3" => CodecConfig::Mp3,
            _ => return None,
        })
    }

    fn track_config(&self, track: &TrackData) -> Option<TrackConfig> {
        let timescale = self.timescale_for(track);
        let codec = self.codec_config(track)?;
        let kind = match track.track_type {
            Some(TrackType::Audio) => MediaKind::Audio {
                sample_rate: timescale,
                channels: track.channels.unwrap_or(2) as u16,
            },
            _ => MediaKind::Video {
                width: track.pixel_width.unwrap_or(0) as u16,
                height: track.pixel_height.unwrap_or(0) as u16,
            },
        };
        // Total duration expressed in this track's timescale.
        let seconds = self.duration_ticks * self.timestamp_scale_ns as f64 / 1e9;
        let duration = (seconds * timescale as f64) as u64;
        Some(TrackConfig {
            track_id: track.track_number.unwrap_or(1) as u32,
            timescale,
            duration,
            language: track.language_tag().to_string(),
            kind,
            codec,
        })
    }

    pub fn init_segment(&self, track_number: u64) -> Option<Vec<u8>> {
        let track = self.track(track_number)?;
        let cfg = self.track_config(track)?;
        Some(build_init_segment(&cfg))
    }

    /// Absolute byte offset of the cluster to start at for `time_ms` on `track`.
    pub fn cue_offset(&self, track_number: u64, time_ms: u64) -> Option<u64> {
        let target_ticks = time_ms * 1_000_000 / self.timestamp_scale_ns.max(1);
        cue_cluster_for_time(&self.cues, track_number, target_ticks)
            .map(|pos| self.segment_data_start + pos)
            .or(self.first_cluster_offset)
    }

    /// Remux `[start_ms, end_ms)` of `track_number` into one fMP4 media segment.
    pub async fn media_segment(
        &self,
        track_number: u64,
        start_ms: u64,
        end_ms: u64,
    ) -> Option<Vec<u8>> {
        let track = self.track(track_number)?.clone();
        let is_audio = track.track_type == Some(TrackType::Audio);
        let start_offset = self.cue_offset(track_number, start_ms)?;

        let frames = self
            .collect_frames(track_number, start_offset, end_ms)
            .await;
        if frames.is_empty() {
            return None;
        }

        let (base, samples) = if is_audio {
            let sample_rate = track.sampling_frequency.unwrap_or(48000.0) as u32;
            let spf = samples_per_frame(track.codec_id.as_deref(), track.codec_private.as_deref());
            audio_samples(&frames, spf, sample_rate, self.timestamp_scale_ns)
        } else {
            let frame_dur = track
                .default_duration
                .map(|ns| (ns / self.timestamp_scale_ns.max(1)) as i64)
                .unwrap_or(1);
            video_samples(&frames, frame_dur)
        };

        let seq = (start_ms as u32).wrapping_add(1);
        Some(build_media_segment(track_number as u32, seq, base, &samples))
    }

    /// Emit a self-contained Matroska chunk for `[start_ms, end_ms)` of an audio
    /// `track_number`, for in-browser transcoding (ffmpeg.wasm) of codecs MSE can't
    /// decode natively. Mirrors [`Self::media_segment`]'s frame collection, but
    /// repackages the raw frames into a minimal MKV (header + Info + Tracks +
    /// Cluster(s)) so ffmpeg — given the codec + CodecPrivate context — can decode and
    /// re-encode them. Returns `(base_seconds, mkv_bytes)`: the chunk's timeline is
    /// zero-anchored (ffmpeg normalizes the first packet to 0 regardless), and
    /// `base_seconds` is its true start on the media timeline so the caller can place
    /// the transcoded fragment with `SourceBuffer.timestampOffset`.
    pub async fn audio_chunk(
        &self,
        track_number: u64,
        start_ms: u64,
        end_ms: u64,
    ) -> Option<(f64, Vec<u8>)> {
        let track = self.track(track_number)?.clone();
        if track.track_type != Some(TrackType::Audio) {
            return None;
        }
        let start_offset = self.cue_offset(track_number, start_ms)?;
        let frames = self.collect_frames(track_number, start_offset, end_ms).await;
        if frames.is_empty() {
            return None;
        }
        let base_seconds = frames[0].pts_ticks.max(0) as f64 * self.timestamp_scale_ns as f64 / 1e9;
        let params = crate::mkv_write::AudioChunkParams {
            timestamp_scale_ns: self.timestamp_scale_ns,
            codec_id: track.codec_id.as_deref().unwrap_or(""),
            codec_private: track.codec_private.as_deref(),
            sample_rate: track.sampling_frequency.unwrap_or(48000.0),
            channels: track.channels.unwrap_or(2),
            bit_depth: track.bit_depth,
            codec_delay_ns: track.codec_delay,
            seek_preroll_ns: track.seek_preroll,
        };
        Some((base_seconds, crate::mkv_write::build_audio_chunk(&params, &frames)))
    }

    /// Walk clusters from `start_offset`, gathering this track's frames (with
    /// absolute PTS in MKV ticks) until a frame's presentation time reaches `end_ms`.
    /// Each cluster is read in a single request and parsed from memory.
    async fn collect_frames(
        &self,
        track_number: u64,
        start_offset: u64,
        end_ms: u64,
    ) -> Vec<TimedFrame> {
        let end_ticks = (end_ms * 1_000_000 / self.timestamp_scale_ns.max(1)) as i64;
        let mut out = Vec::new();
        let mut clusters = self.iter_at(start_offset, Some(self.segment_end));

        // Tile on whole clusters. A cluster is a complete, keyframe-bounded set of
        // GOPs, so taking it whole avoids splitting reordered (B-frame) sequences —
        // which would leave holes in the presentation timeline and stall the decoder.
        // We peek each cluster's timestamp (a tiny read) and stop at the first cluster
        // that begins at or after end_ticks, so the boundary cluster is never fetched.
        while let Some(el) = clusters.next().await {
            if el.id != ID_CLUSTER {
                continue;
            }
            let EbmlPayload::Master(cluster) = el.payload else {
                continue;
            };
            let cstart = cluster.current;
            let cend = cluster.end.unwrap_or(self.segment_end);
            let cluster_time = self.peek_cluster_time(cstart, cend).await;
            if !out.is_empty() && cluster_time >= end_ticks {
                break;
            }
            let buffered = self.buffer_range(cstart, cend).await;
            collect_cluster_frames(buffered, track_number, &mut out).await;
        }
        out
    }

    /// Read just a cluster's leading bytes to recover its `Timestamp` without
    /// buffering the whole cluster (the timestamp precedes any blocks).
    async fn peek_cluster_time(&self, cstart: u64, cend: u64) -> i64 {
        let peek_end = (cstart + 256).min(cend);
        let mut it = self.buffer_range(cstart, peek_end).await;
        while let Some(child) = it.next().await {
            match child.id {
                ID_TIMESTAMP => {
                    if let EbmlPayload::UnsignedInt(v) = child.payload {
                        return v as i64;
                    }
                }
                ID_SIMPLEBLOCK | ID_BLOCKGROUP => break,
                _ => {}
            }
        }
        0
    }

    /// Collect all subtitle cues for a track (full cluster scan) as a WebVTT doc.
    pub async fn subtitles(&self, track_number: u64) -> Option<String> {
        let track = self.track(track_number)?;
        if track.track_type != Some(TrackType::Subtitle) {
            return None;
        }
        let start = self.first_cluster_offset?;
        let cues = self.collect_subtitle_cues(track_number, start).await;
        Some(cues_to_webvtt(&cues))
    }

    async fn collect_subtitle_cues(&self, track_number: u64, start: u64) -> Vec<SubtitleCue> {
        let mut cues = Vec::new();
        let mut clusters = self.iter_at(start, Some(self.segment_end));
        while let Some(el) = clusters.next().await {
            if el.id != ID_CLUSTER {
                continue;
            }
            let EbmlPayload::Master(cluster) = el.payload else {
                continue;
            };
            let buffered = self
                .buffer_range(cluster.current, cluster.end.unwrap_or(self.segment_end))
                .await;
            walk_cluster_subtitles(buffered, track_number, self.timestamp_scale_ns, &mut cues).await;
        }
        cues
    }

    /// JSON list of font attachments with the **absolute** byte range of each font's
    /// data, e.g. `[{"name":"x.ttf","mime":"font/ttf","uid":1,"start":1234,"end":5678}]`.
    /// JS fetches each range out-of-band (a separate connection) into a Blob for libass.
    pub fn font_attachments_json(&self) -> String {
        let mut items = Vec::new();
        for a in &self.attachments {
            if !is_font_attachment(&a.mime, &a.name) {
                continue;
            }
            items.push(format!(
                "{{\"name\":\"{}\",\"mime\":\"{}\",\"uid\":{},\"start\":{},\"end\":{}}}",
                json_escape(&a.name),
                json_escape(&a.mime),
                a.uid,
                a.data_start,
                a.data_end,
            ));
        }
        format!("[{}]", items.join(","))
    }

    /// JSON list of chapters (chosen edition), each with all localized titles so JS can
    /// pick one by the selected audio language:
    /// `[{"startMs":N,"endMs":N|null,"uid":N,"displays":[{"text":"…","language":"eng","languageBcp47":"en"}]}]`.
    pub fn chapters_json(&self) -> String {
        let mut items = Vec::new();
        for c in &self.chapters {
            let displays: Vec<String> = c
                .displays
                .iter()
                .map(|d| {
                    format!(
                        "{{\"text\":\"{}\",\"language\":\"{}\",\"languageBcp47\":{}}}",
                        json_escape(&d.text),
                        json_escape(&d.language),
                        match &d.language_bcp47 {
                            Some(l) => format!("\"{}\"", json_escape(l)),
                            None => "null".to_string(),
                        },
                    )
                })
                .collect();
            items.push(format!(
                "{{\"uid\":{},\"startMs\":{},\"endMs\":{},\"displays\":[{}]}}",
                c.uid,
                c.start_ms,
                match c.end_ms {
                    Some(e) => e.to_string(),
                    None => "null".to_string(),
                },
                displays.join(","),
            ));
        }
        format!("[{}]", items.join(","))
    }

    /// The ASS/SSA script header (`[Script Info]` … `[Events]` Format line) for a
    /// subtitle track — its `CodecPrivate`. This is the document libass parses first;
    /// the per-line dialogue events are streamed in separately via [`Self::subtitle_events_json`].
    pub fn subtitle_header(&self, track_number: u64) -> Option<String> {
        let track = self.track(track_number)?;
        if track.track_type != Some(TrackType::Subtitle) {
            return None;
        }
        track
            .codec_private
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// Subtitle cues whose blocks fall in `[start_ms, end_ms)` of `track_number`, as JSON
    /// `[{"start":ms,"end":ms,"text":"ReadOrder,Layer,Style,…,Text"}]`. Walks only the
    /// clusters covering that window (the same clusters video already fetched, so mostly
    /// cache hits) — never the whole file. `text` is the raw MKV block payload; JS turns
    /// it into a `Dialogue:` line and dedups on its leading ReadOrder field.
    pub async fn subtitle_events_json(&self, track_number: u64, start_ms: u64, end_ms: u64) -> String {
        let is_sub = self
            .track(track_number)
            .map(|t| t.track_type == Some(TrackType::Subtitle))
            .unwrap_or(false);
        if !is_sub {
            return "[]".to_string();
        }
        let Some(start_offset) = self.cue_offset(track_number, start_ms) else {
            return "[]".to_string();
        };
        let cues = self
            .collect_subtitle_cues_range(track_number, start_offset, end_ms)
            .await;
        let items: Vec<String> = cues
            .iter()
            .map(|c| {
                format!(
                    "{{\"start\":{},\"end\":{},\"text\":\"{}\"}}",
                    c.start_ms,
                    c.end_ms,
                    json_escape(&c.text)
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }

    /// Like [`Self::collect_subtitle_cues`] but bounded: walks clusters from `start_offset`
    /// and stops at the first cluster beginning at/after `end_ms` (peeked cheaply), so a
    /// window's events come from only the clusters that cover it.
    async fn collect_subtitle_cues_range(
        &self,
        track_number: u64,
        start_offset: u64,
        end_ms: u64,
    ) -> Vec<SubtitleCue> {
        let end_ticks = (end_ms * 1_000_000 / self.timestamp_scale_ns.max(1)) as i64;
        let mut cues = Vec::new();
        let mut clusters = self.iter_at(start_offset, Some(self.segment_end));
        while let Some(el) = clusters.next().await {
            if el.id != ID_CLUSTER {
                continue;
            }
            let EbmlPayload::Master(cluster) = el.payload else {
                continue;
            };
            let cstart = cluster.current;
            let cend = cluster.end.unwrap_or(self.segment_end);
            let cluster_time = self.peek_cluster_time(cstart, cend).await;
            if !cues.is_empty() && cluster_time >= end_ticks {
                break;
            }
            let buffered = self.buffer_range(cstart, cend).await;
            walk_cluster_subtitles(buffered, track_number, self.timestamp_scale_ns, &mut cues).await;
        }
        cues
    }

    pub fn duration_ms(&self) -> u64 {
        (self.duration_ticks * self.timestamp_scale_ns as f64 / 1e6) as u64
    }

    /// Cue presentation times in milliseconds for the **video** track only, the real
    /// keyframe/cluster boundaries JS should tile segments on. Files often index cues
    /// for every track (frequent, mid-cluster audio cues), which would otherwise make
    /// thousands of tiny overlapping segments. Deduplicated and ascending. Empty when
    /// the file has no usable Cues.
    pub fn cue_times_ms(&self) -> Vec<u64> {
        let video_track = self
            .tracks
            .iter()
            .find(|t| t.track_type == Some(TrackType::Video))
            .and_then(|t| t.track_number);

        let mut out: Vec<u64> = Vec::new();
        for c in &self.cues {
            let indexes_video = match video_track {
                Some(vt) => c.positions.iter().any(|p| p.track == vt),
                None => true,
            };
            if !indexes_video {
                continue;
            }
            let ms = (c.time as u128 * self.timestamp_scale_ns as u128 / 1_000_000) as u64;
            if out.last() != Some(&ms) {
                out.push(ms);
            }
        }
        out
    }

    pub fn timestamp_scale(&self) -> u64 {
        self.timestamp_scale_ns
    }

    /// JSON description of all tracks for the JS side.
    pub fn tracks_json(&self) -> String {
        let mut items = Vec::new();
        for t in &self.tracks {
            let kind = t.track_type.map(|k| k.as_str()).unwrap_or("unknown");
            let codec_id = t.codec_id.as_deref().unwrap_or("");
            let codec_string = t.codec_string().unwrap_or_default();
            let mime = t.mime_type().unwrap_or_default();
            // Prefer the human-readable track title (`Name`); fall back to `CodecName`.
            let name = t
                .name
                .clone()
                .or_else(|| t.codec_name.clone())
                .unwrap_or_default();
            items.push(format!(
                "{{\"number\":{},\"type\":\"{}\",\"codec_id\":\"{}\",\"codec_string\":\"{}\",\"mime\":\"{}\",\"language\":\"{}\",\"name\":\"{}\",\"default\":{},\"forced\":{}}}",
                t.track_number.unwrap_or(0),
                kind,
                json_escape(codec_id),
                json_escape(&codec_string),
                json_escape(&mime),
                json_escape(t.language_tag()),
                json_escape(&name),
                t.flag_default,
                t.flag_forced,
            ));
        }
        format!("[{}]", items.join(","))
    }
}

// ----------------------------------------------------------------------------
// Generic in-memory cluster walking (works over MemSource or any EbmlSource).
// ----------------------------------------------------------------------------

/// Append all of a block's frames to `out` at the block's absolute PTS.
fn push_block(out: &mut Vec<TimedFrame>, block: &BlockFrames, cluster_time: i64) {
    let pts = cluster_time + block.rel_timecode as i64;
    for frame in &block.frames {
        out.push(TimedFrame {
            pts_ticks: pts,
            data: frame.clone(),
            is_keyframe: block.is_keyframe,
        });
    }
}

/// Collect every `track_number` frame in one (already-buffered) cluster.
async fn collect_cluster_frames<M: EbmlSource + PartialEq + Clone>(
    mut cluster: EbmlIterator<M>,
    track_number: u64,
    out: &mut Vec<TimedFrame>,
) {
    let mut cluster_time: i64 = 0;
    while let Some(child) = cluster.next().await {
        match child.payload {
            EbmlPayload::UnsignedInt(v) if child.id == ID_TIMESTAMP => cluster_time = v as i64,
            EbmlPayload::Binary((start, end)) if child.id == ID_SIMPLEBLOCK => {
                let bytes = cluster.read_range(start, end).await;
                if let Some(block) = parse_block(&bytes, true, false) {
                    if block.track_number == track_number {
                        push_block(out, &block, cluster_time);
                    }
                }
            }
            EbmlPayload::Master(group) if child.id == ID_BLOCKGROUP => {
                if let Some(block) = read_block_group(group, track_number).await {
                    push_block(out, &block, cluster_time);
                }
            }
            _ => {}
        }
    }
}

async fn read_block_group<M: EbmlSource + PartialEq + Clone>(
    mut group: EbmlIterator<M>,
    track_number: u64,
) -> Option<BlockFrames> {
    let mut block_range: Option<(u64, u64)> = None;
    let mut has_reference = false;
    while let Some(field) = group.next().await {
        match field.payload {
            EbmlPayload::Binary((start, end)) if field.id == ID_BLOCK => {
                block_range = Some((start, end));
            }
            _ if field.id == ID_REFERENCEBLOCK => has_reference = true,
            _ => {}
        }
    }
    let (start, end) = block_range?;
    let bytes = group.read_range(start, end).await;
    let mut block = parse_block(&bytes, false, !has_reference)?;
    if block.track_number != track_number {
        return None;
    }
    block.is_keyframe = !has_reference;
    Some(block)
}

/// Walk one cluster's children, collecting `track_number`'s subtitle cues.
async fn walk_cluster_subtitles<M: EbmlSource + PartialEq + Clone>(
    mut cluster: EbmlIterator<M>,
    track_number: u64,
    scale_ns: u64,
    cues: &mut Vec<SubtitleCue>,
) {
    let to_ms = |ticks: i64| -> u64 { (ticks.max(0) as u128 * scale_ns as u128 / 1_000_000) as u64 };
    let mut cluster_time: i64 = 0;
    while let Some(child) = cluster.next().await {
        match child.payload {
            EbmlPayload::UnsignedInt(v) if child.id == ID_TIMESTAMP => cluster_time = v as i64,
            EbmlPayload::Binary((s, e)) if child.id == ID_SIMPLEBLOCK => {
                let bytes = cluster.read_range(s, e).await;
                if let Some(b) = parse_block(&bytes, true, false) {
                    if b.track_number == track_number {
                        let start_ms = to_ms(cluster_time + b.rel_timecode as i64);
                        for f in &b.frames {
                            cues.push(SubtitleCue {
                                start_ms,
                                end_ms: start_ms + 4000,
                                text: String::from_utf8_lossy(f).into_owned(),
                            });
                        }
                    }
                }
            }
            EbmlPayload::Master(mut group) if child.id == ID_BLOCKGROUP => {
                let mut block_range = None;
                let mut duration_ticks: Option<u64> = None;
                while let Some(gf) = group.next().await {
                    match gf.payload {
                        EbmlPayload::Binary((s, e)) if gf.id == ID_BLOCK => block_range = Some((s, e)),
                        EbmlPayload::UnsignedInt(v) if gf.id == ID_BLOCKDURATION => {
                            duration_ticks = Some(v)
                        }
                        _ => {}
                    }
                }
                if let Some((s, e)) = block_range {
                    let bytes = group.read_range(s, e).await;
                    if let Some(b) = parse_block(&bytes, false, true) {
                        if b.track_number == track_number {
                            let start_ms = to_ms(cluster_time + b.rel_timecode as i64);
                            let dur_ms = duration_ticks.map(|d| to_ms(d as i64)).unwrap_or(4000);
                            for f in &b.frames {
                                cues.push(SubtitleCue {
                                    start_ms,
                                    end_ms: start_ms + dur_ms,
                                    text: String::from_utf8_lossy(f).into_owned(),
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse an `Attachments` master into [`Attachment`]s. Iterates each `AttachedFile`'s
/// fields, recording the `FileData` *range* (never its bytes), so this is cheap even
/// over the network — only the small name/mime fields are actually read.
async fn parse_attachments<S>(mut atts: EbmlIterator<S>) -> Vec<Attachment>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut out = Vec::new();
    while let Some(el) = atts.next().await {
        if el.id != ID_ATTACHEDFILE {
            continue;
        }
        let EbmlPayload::Master(mut file) = el.payload else {
            continue;
        };
        let mut name: Option<String> = None;
        let mut mime: Option<String> = None;
        let mut uid: Option<u64> = None;
        let mut data: Option<(u64, u64)> = None;
        while let Some(field) = file.next().await {
            match field.payload {
                EbmlPayload::String(s) if field.id == ID_FILENAME => name = Some(s),
                EbmlPayload::String(s) if field.id == ID_FILEMEDIATYPE => mime = Some(s),
                EbmlPayload::UnsignedInt(v) if field.id == ID_FILEUID => uid = Some(v),
                EbmlPayload::Binary((start, end)) if field.id == ID_FILEDATA => {
                    data = Some((start, end));
                }
                _ => {}
            }
        }
        if let Some((data_start, data_end)) = data {
            out.push(Attachment {
                name: name.unwrap_or_default(),
                mime: mime.unwrap_or_default(),
                uid: uid.unwrap_or(0),
                data_start,
                data_end,
            });
        }
    }
    out
}

/// Parse a `Chapters` master into the chosen edition's top-level chapters. Prefers the
/// edition flagged default, else the first non-hidden one. Nested chapters are not
/// flattened (rare for playback menus).
async fn parse_chapters<S>(mut chapters: EbmlIterator<S>) -> Vec<ChapterEntry>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut default_edition: Option<Vec<ChapterEntry>> = None;
    let mut first_edition: Option<Vec<ChapterEntry>> = None;
    while let Some(el) = chapters.next().await {
        if el.id != ID_EDITIONENTRY {
            continue;
        }
        let EbmlPayload::Master(mut ed) = el.payload else {
            continue;
        };
        let mut is_default = false;
        let mut is_hidden = false;
        let mut atoms = Vec::new();
        while let Some(field) = ed.next().await {
            match field.payload {
                EbmlPayload::UnsignedInt(v) if field.id == ID_EDITIONFLAGDEFAULT => is_default = v != 0,
                EbmlPayload::UnsignedInt(v) if field.id == ID_EDITIONFLAGHIDDEN => is_hidden = v != 0,
                EbmlPayload::Master(atom) if field.id == ID_CHAPTERATOM => {
                    if let Some(c) = parse_chapter_atom(atom).await {
                        atoms.push(c);
                    }
                }
                _ => {}
            }
        }
        if is_hidden || atoms.is_empty() {
            continue;
        }
        atoms.sort_by_key(|c| c.start_ms);
        if is_default {
            default_edition.get_or_insert(atoms);
        } else if first_edition.is_none() {
            first_edition = Some(atoms);
        }
    }
    default_edition.or(first_edition).unwrap_or_default()
}

async fn parse_chapter_atom<S>(mut atom: EbmlIterator<S>) -> Option<ChapterEntry>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut uid = 0u64;
    let mut start_ns: Option<u64> = None;
    let mut end_ns: Option<u64> = None;
    let mut hidden = false;
    let mut displays = Vec::new();
    while let Some(field) = atom.next().await {
        match field.payload {
            EbmlPayload::UnsignedInt(v) if field.id == ID_CHAPTERUID => uid = v,
            EbmlPayload::UnsignedInt(v) if field.id == ID_CHAPTERTIMESTART => start_ns = Some(v),
            EbmlPayload::UnsignedInt(v) if field.id == ID_CHAPTERTIMEEND => end_ns = Some(v),
            EbmlPayload::UnsignedInt(v) if field.id == ID_CHAPTERFLAGHIDDEN => hidden = v != 0,
            EbmlPayload::Master(disp) if field.id == ID_CHAPTERDISPLAY => {
                if let Some(d) = parse_chapter_display(disp).await {
                    displays.push(d);
                }
            }
            _ => {}
        }
    }
    if hidden {
        return None;
    }
    // ChapterTimeStart/End are absolute nanoseconds (not TimestampScale-scaled).
    let start_ns = start_ns?;
    Some(ChapterEntry {
        uid,
        start_ms: start_ns / 1_000_000,
        end_ms: end_ns.map(|n| n / 1_000_000),
        displays,
    })
}

async fn parse_chapter_display<S>(mut disp: EbmlIterator<S>) -> Option<ChapterDisplay>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut text: Option<String> = None;
    let mut language: Option<String> = None;
    let mut language_bcp47: Option<String> = None;
    while let Some(field) = disp.next().await {
        match field.payload {
            EbmlPayload::String(s) if field.id == ID_CHAPSTRING => text = Some(s),
            EbmlPayload::String(s) if field.id == ID_CHAPLANGUAGE => language = Some(s),
            EbmlPayload::String(s) if field.id == ID_CHAPLANGUAGEBCP47 => language_bcp47 = Some(s),
            _ => {}
        }
    }
    Some(ChapterDisplay {
        text: text?,
        language: language.unwrap_or_else(|| "eng".to_string()),
        language_bcp47,
    })
}

fn samples_per_frame(codec_id: Option<&str>, codec_private: Option<&[u8]>) -> u32 {
    match codec_id {
        Some("A_OPUS") => 960,
        Some("A_AC3") | Some("A_EAC3") => 1536,
        // MPEG-1 Layer III is 1152 samples/frame (the common case; MPEG-2/2.5 use
        // 576, which we don't distinguish here — per-segment PTS re-anchors any drift).
        Some("A_MPEG/L3") => 1152,
        // FLAC block size is per-stream; read it from STREAMINFO, default 4096.
        Some("A_FLAC") => flac_block_size(codec_private).unwrap_or(4096),
        _ => 1024, // AAC and default
    }
}

/// FLAC frame size (samples) from the STREAMINFO max-blocksize field in the MKV
/// `CodecPrivate`. Layout: optional `fLaC` marker (4), metadata block header (4),
/// then STREAMINFO data starting with min_blocksize(2) and max_blocksize(2), both
/// big-endian. For the common fixed-block stream min == max.
fn flac_block_size(codec_private: Option<&[u8]>) -> Option<u32> {
    let cp = codec_private?;
    let streaminfo = if cp.len() >= 4 && &cp[0..4] == b"fLaC" { 8 } else { 4 };
    let max = cp.get(streaminfo + 2..streaminfo + 4)?;
    let v = u16::from_be_bytes([max[0], max[1]]) as u32;
    (v > 0).then_some(v)
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ============================================================================
// wasm-bindgen facade
// ============================================================================

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use crate::stream_source::StreamSource;
    use ebml_wasm::matroska_data::element_id_type_map;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct MatroskaPlayer(Demuxer<StreamSource>);

    /// A transcoding input chunk: a self-contained Matroska blob plus its true start
    /// time on the media timeline. JS transcodes `data` with ffmpeg.wasm and appends
    /// the result at `base_seconds` via `SourceBuffer.timestampOffset`.
    #[wasm_bindgen]
    pub struct AudioChunk {
        base_seconds: f64,
        data: Vec<u8>,
    }

    #[wasm_bindgen]
    impl AudioChunk {
        #[wasm_bindgen(getter)]
        pub fn base_seconds(&self) -> f64 {
            self.base_seconds
        }
        /// The Matroska bytes (moved out; call once).
        #[wasm_bindgen(getter)]
        pub fn data(&mut self) -> Box<[u8]> {
            std::mem::take(&mut self.data).into_boxed_slice()
        }
    }

    #[wasm_bindgen]
    impl MatroskaPlayer {
        pub async fn open(url: String) -> MatroskaPlayer {
            let source = StreamSource::new(url);
            source.prefetch().await;
            let ebml = Ebml::new(source, element_id_type_map());
            MatroskaPlayer(Demuxer::open(ebml).await)
        }

        pub fn tracks(&self) -> String {
            self.0.tracks_json()
        }

        pub fn init_segment(&self, track_number: u64) -> Option<Box<[u8]>> {
            self.0.init_segment(track_number).map(|v| v.into_boxed_slice())
        }

        pub async fn media_segment(
            &self,
            track_number: u64,
            start_ms: u64,
            end_ms: u64,
        ) -> Option<Box<[u8]>> {
            self.0
                .media_segment(track_number, start_ms, end_ms)
                .await
                .map(|v| v.into_boxed_slice())
        }

        /// Self-contained Matroska chunk for `[start_ms, end_ms)` of an audio track,
        /// for in-browser transcoding (see [`Demuxer::audio_chunk`]). Returns an
        /// [`AudioChunk`] (bytes + base time) or `None` for non-audio / empty windows.
        pub async fn audio_chunk(
            &self,
            track_number: u64,
            start_ms: u64,
            end_ms: u64,
        ) -> Option<AudioChunk> {
            self.0
                .audio_chunk(track_number, start_ms, end_ms)
                .await
                .map(|(base_seconds, data)| AudioChunk { base_seconds, data })
        }

        pub fn cue_offset(&self, track_number: u64, time_ms: u64) -> Option<u64> {
            self.0.cue_offset(track_number, time_ms)
        }

        pub async fn subtitles(&self, track_number: u64) -> Option<String> {
            self.0.subtitles(track_number).await
        }

        /// JSON list of font attachments (name, mime, uid, absolute data byte range).
        pub fn font_attachments(&self) -> String {
            self.0.font_attachments_json()
        }

        /// JSON list of chapters (chosen edition) with all localized titles.
        pub fn chapters(&self) -> String {
            self.0.chapters_json()
        }

        /// The ASS/SSA header (`CodecPrivate`) for a subtitle track, or `None`.
        pub fn subtitle_header(&self, track_number: u64) -> Option<String> {
            self.0.subtitle_header(track_number)
        }

        /// JSON list of subtitle cues for `[start_ms, end_ms)` of `track_number`.
        pub async fn subtitle_events(&self, track_number: u64, start_ms: u64, end_ms: u64) -> String {
            self.0.subtitle_events_json(track_number, start_ms, end_ms).await
        }

        pub fn duration_ms(&self) -> u64 {
            self.0.duration_ms()
        }

        pub fn timestamp_scale(&self) -> u64 {
            self.0.timestamp_scale()
        }

        /// JSON array of cue times in ms (segment boundaries), e.g. `[0,2000,...]`.
        pub fn cue_times(&self) -> String {
            let times = self.0.cue_times_ms();
            let items: Vec<String> = times.iter().map(|t| t.to_string()).collect();
            format!("[{}]", items.join(","))
        }
    }

}
