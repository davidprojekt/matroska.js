//! Segment positioning: the Segment content-start offset (the base for every
//! `SeekPosition` / `CueClusterPosition`), the `SeekHead` element index, and the
//! `Cues` time→byte index used for seeking.

use crate::ebml::{EbmlIterator, EbmlPayload, EbmlSource};
use crate::matroska_data::{
    ID_CUECLUSTERPOSITION, ID_CUEPOINT, ID_CUERELATIVEPOSITION, ID_CUETIME, ID_CUETRACK,
    ID_CUETRACKPOSITIONS, ID_SEEK, ID_SEEKID, ID_SEEKPOSITION,
};

/// One `SeekHead` entry: which top-level element, and where it lives (byte offset
/// **relative to the Segment content start**).
#[derive(Debug, Clone, Copy)]
pub struct SeekEntry {
    pub seek_id: u64,
    pub position: u64,
}

/// One cue track position within a `CuePoint`.
#[derive(Debug, Clone, Copy)]
pub struct CueTrackPosition {
    pub track: u64,
    /// Byte offset of the target Cluster, relative to the Segment content start.
    pub cluster_position: u64,
    /// Optional byte offset of the block within the Cluster.
    pub relative_position: Option<u64>,
}

/// One `CuePoint`: a presentation time and the per-track cluster positions for it.
#[derive(Debug, Clone)]
pub struct CuePoint {
    /// Time in `TimestampScale` units (raw `CueTime`).
    pub time: u64,
    pub positions: Vec<CueTrackPosition>,
}

/// Parse a `SeekHead` master iterator into its entries.
pub async fn parse_seek_head<S>(mut seek_head: EbmlIterator<S>) -> Vec<SeekEntry>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut entries = Vec::new();
    while let Some(el) = seek_head.next().await {
        if el.id != ID_SEEK {
            continue;
        }
        let EbmlPayload::Master(mut seek) = el.payload else {
            continue;
        };
        let mut seek_id: Option<u64> = None;
        let mut position: Option<u64> = None;
        while let Some(field) = seek.next().await {
            match field.payload {
                EbmlPayload::Binary((start, end)) if field.id == ID_SEEKID => {
                    let bytes = seek.read_range(start, end).await;
                    seek_id = Some(bytes_to_u64(&bytes));
                }
                EbmlPayload::UnsignedInt(v) if field.id == ID_SEEKPOSITION => position = Some(v),
                _ => {}
            }
        }
        if let (Some(seek_id), Some(position)) = (seek_id, position) {
            entries.push(SeekEntry { seek_id, position });
        }
    }
    entries
}

/// Parse a `Cues` master iterator into a time-ordered list of cue points.
pub async fn parse_cues<S>(mut cues: EbmlIterator<S>) -> Vec<CuePoint>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut points = Vec::new();
    while let Some(el) = cues.next().await {
        if el.id != ID_CUEPOINT {
            continue;
        }
        let EbmlPayload::Master(mut point) = el.payload else {
            continue;
        };
        let mut time: Option<u64> = None;
        let mut positions = Vec::new();
        while let Some(field) = point.next().await {
            match field.payload {
                EbmlPayload::UnsignedInt(v) if field.id == ID_CUETIME => time = Some(v),
                EbmlPayload::Master(ctp) if field.id == ID_CUETRACKPOSITIONS => {
                    if let Some(pos) = parse_cue_track_positions(ctp).await {
                        positions.push(pos);
                    }
                }
                _ => {}
            }
        }
        if let Some(time) = time {
            points.push(CuePoint { time, positions });
        }
    }
    points.sort_by_key(|p| p.time);
    points
}

async fn parse_cue_track_positions<S>(mut ctp: EbmlIterator<S>) -> Option<CueTrackPosition>
where
    S: EbmlSource + PartialEq + Clone,
{
    let mut track: Option<u64> = None;
    let mut cluster_position: Option<u64> = None;
    let mut relative_position: Option<u64> = None;
    while let Some(field) = ctp.next().await {
        if let EbmlPayload::UnsignedInt(v) = field.payload {
            match field.id {
                ID_CUETRACK => track = Some(v),
                ID_CUECLUSTERPOSITION => cluster_position = Some(v),
                ID_CUERELATIVEPOSITION => relative_position = Some(v),
                _ => {}
            }
        }
    }
    Some(CueTrackPosition {
        track: track?,
        cluster_position: cluster_position?,
        relative_position,
    })
}

/// Big-endian fold of up to 8 bytes (used for `SeekID`, which stores the full EBML
/// element ID including its length-descriptor bits).
fn bytes_to_u64(bytes: &[u8]) -> u64 {
    let mut value: u64 = 0;
    for &b in bytes.iter().take(8) {
        value = (value << 8) | b as u64;
    }
    value
}

/// Find the cue whose time is the greatest `<= target_time` (raw TimestampScale
/// units), returning its cluster position for `track` (relative to Segment content
/// start). Falls back to the first cue's position when the target precedes all cues.
pub fn cue_cluster_for_time(
    cues: &[CuePoint],
    track: u64,
    target_time: u64,
) -> Option<u64> {
    let mut chosen: Option<u64> = None;
    for point in cues {
        if point.time > target_time {
            break;
        }
        if let Some(pos) = point
            .positions
            .iter()
            .find(|p| p.track == track)
            .or_else(|| point.positions.first())
        {
            chosen = Some(pos.cluster_position);
        }
    }
    chosen.or_else(|| {
        cues.first()
            .and_then(|p| p.positions.first())
            .map(|p| p.cluster_position)
    })
}

/// Cluster position of the first cue whose time is `>= target_time` — i.e. the start
/// of the next segment boundary. Used as an exclusive end offset when collecting a
/// time range, so we never read past it. `None` when the target is beyond all cues.
pub fn cue_cluster_at_or_after(cues: &[CuePoint], track: u64, target_time: u64) -> Option<u64> {
    for point in cues {
        if point.time < target_time {
            continue;
        }
        return point
            .positions
            .iter()
            .find(|p| p.track == track)
            .or_else(|| point.positions.first())
            .map(|p| p.cluster_position);
    }
    None
}
