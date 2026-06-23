use std::collections::HashMap;
use crate::ebml::{EbmlIterator, EbmlPayload};
use crate::fs_source::FsSource;
use crate::matroska_data::{ID_CLUSTER, ID_CODECID, ID_CODECNAME, ID_CODECPRIVATE, ID_FRAMERATE, ID_PIXELHEIGHT, ID_PIXELWIDTH, ID_SIMPLEBLOCK, ID_TIMESTAMP, ID_TRACKENTRY, ID_TRACKNUMBER, ID_TRACKS, ID_TRACKTYPE, ID_TRACKUID};


pub enum TrackType {
    VIDEO = 1,
    AUDIO = 2,
    COMPLEX = 3,
    LOGO = 16,
    SUBTITLE = 17,
    BUTTONS = 18,
    CONTROL = 32,
    METADATA = 33,
}

impl TryFrom<u8> for TrackType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(TrackType::VIDEO),
            2 => Ok(TrackType::AUDIO),
            3 => Ok(TrackType::COMPLEX),
            16 => Ok(TrackType::LOGO),
            17 => Ok(TrackType::SUBTITLE),
            18 => Ok(TrackType::BUTTONS),
            32 => Ok(TrackType::CONTROL),
            33 => Ok(TrackType::METADATA),
            _ => Err(()),
        }
    }
}

#[derive(Default)]
pub struct TrackData {
    pub track_number: Option<u64>,
    pub track_uid: Option<u64>,
    pub track_type: Option<TrackType>,

    pub codec_id: Option<String>,
    pub codec_private: Option<Box<[u8]>>,
    pub codec_name: Option<String>,

    pub pixel_width: Option<u64>,
    pub pixel_height: Option<u64>,
    pub frame_rate: Option<f64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lacing {
    NoLacing,
    Xiph,
    FixedSize,
    Ebml,
}

#[derive(Debug)]
pub struct FrameFlags {
    pub keyframe: bool,
    pub invisible: bool,
    pub lacing: Lacing,
    pub discardable: bool,
}

impl FrameFlags {
    pub fn parse(flags: u8) -> Self {
        // Bit 7: Mask 1000_0000 (0x80) - Keyframe; Set when the Block contains only keyframes.
        let keyframe = (flags & 0b1000_0000) != 0;

        // Bit 3: Mask 0000_1000 (0x08) - Invisible; the codec SHOULD decode this frame but not display it.
        let invisible = (flags & 0b0000_1000) != 0;

        // Bits 2-1: Mask 0000_0110 (0x06), then shift right by 1 to get a 0-3 value
        let lacing_bits = (flags & 0b0000_0110) >> 1;
        let lacing = match lacing_bits {
            0b00 => Lacing::NoLacing,
            0b01 => Lacing::Xiph,
            0b10 => Lacing::FixedSize,
            0b11 => Lacing::Ebml,
            _ => unreachable!(), // A 2-bit value can only be 0, 1, 2, or 3
        };

        // Bit 0: Mask 0000_0001 (0x01) - Discardable; The frames of the Block can be discarded during playing if needed.
        let discardable = (flags & 0b0000_0001) != 0;

        Self {
            keyframe,
            invisible,
            lacing,
            discardable,
        }
    }
}

pub async fn read_blocks(mut file: EbmlIterator<FsSource>) {
    // let mut file = file.peekable();

    let mut tracks: HashMap<u64, TrackData> = HashMap::new();

    while let Some(element) = file.next().await {

        if element.id == ID_TRACKS
            && let EbmlPayload::Master(mut payload) = element.payload
        {
            while let Some(tracks_content) = payload.next().await {
                if tracks_content.id == ID_TRACKENTRY
                    && let EbmlPayload::Master(mut payload) = tracks_content.payload
                {
                    let mut track = TrackData::default();

                    while let Some(trackentry_content) = payload.next().await {
                        if trackentry_content.id == ID_TRACKNUMBER
                            && let EbmlPayload::UnsignedInt(track_number) = trackentry_content.payload
                        {
                            track.track_number = Some(track_number);
                        }
                        else if trackentry_content.id == ID_TRACKUID
                            && let EbmlPayload::UnsignedInt(track_uid) = trackentry_content.payload
                        {
                            track.track_uid = Some(track_uid);
                        }
                        else if trackentry_content.id == ID_TRACKTYPE
                            && let EbmlPayload::UnsignedInt(track_type) = trackentry_content.payload
                            && let Ok(track_type) = (track_type as u8).try_into()
                        {
                            track.track_type = Some(track_type);
                        }
                        else if trackentry_content.id == ID_CODECID
                            && let EbmlPayload::String(codec_id) = trackentry_content.payload
                        {
                            track.codec_id = Some(codec_id);
                        }
                        else if trackentry_content.id == ID_CODECPRIVATE
                            && let EbmlPayload::Binary((start, end)) = trackentry_content.payload
                        {
                            track.codec_private = Some(file.read_range(start, end).await);
                        }
                        else if trackentry_content.id == ID_CODECNAME
                            && let EbmlPayload::String(codec_name) = trackentry_content.payload
                        {
                            track.codec_name = Some(codec_name);
                        }
                        else if trackentry_content.id == ID_PIXELWIDTH
                            && let EbmlPayload::UnsignedInt(pixel_width) = trackentry_content.payload
                        {
                            track.pixel_width = Some(pixel_width);
                        }
                        else if trackentry_content.id == ID_PIXELHEIGHT
                            && let EbmlPayload::UnsignedInt(pixel_height) = trackentry_content.payload
                        {
                            track.pixel_height = Some(pixel_height);
                        }
                        else if trackentry_content.id == ID_FRAMERATE
                            && let EbmlPayload::Float(frame_rate) = trackentry_content.payload
                        {
                            track.frame_rate = Some(frame_rate);
                        }

                        // Maybe: FlagInterlaced, FieldOrder
                    }

                    if let Some(track_number) = track.track_number {
                        tracks.insert(track_number, track);
                    }
                }
            }
        }
        else if element.id == ID_CLUSTER
            && let EbmlPayload::Master(mut payload) = element.payload
        {
            let mut timestamp: u64 = 0;
            while let Some(cluster_content) = payload.next().await {
                if cluster_content.id == ID_TIMESTAMP
                    && let EbmlPayload::UnsignedInt(timestamp_data) = cluster_content.payload
                {
                    timestamp = timestamp_data;
                }

                if cluster_content.id == ID_SIMPLEBLOCK
                    && let EbmlPayload::Binary((mut position, end)) = cluster_content.payload
                {
                    // let data = file.read_range(position, end).await;
                    let (octet_length, track_number) = file.ebml.read_variable_size_uint(position).await;
                    position += octet_length;

                    // if we find frame data for a track that doesn't exist, we should skip it.
                    let Some(track) = tracks.get(&track_number) else {
                        continue;
                    };


                    // Timecode - 2 bytes, signed int - this is relative to the timestamp of the cluster
                    let timecode: i16 = {
                        let raw_bytes = file.read_range(position, position + 1).await.to_vec();
                        position += 2;

                        let mut bytes: [u8; 2] = [0; 2];
                        bytes[..].copy_from_slice(raw_bytes.as_slice());
                        i16::from_be_bytes(bytes)
                    };

                    let absolute_timestamp = timestamp as i128 + timecode as i128;
                    let absolute_timestamp = absolute_timestamp as u64;

                    // for now we expect the files to only have 1 video track, so we just assume we have the right track if its video.
                    // for testing purposes that is.

                    // https://www.ietf.org/rfc/rfc9559.html#name-block-lacing
                    let flags = {
                        let raw_bytes = file.read_range(position, position).await;
                        position += 1;

                        FrameFlags::parse(raw_bytes[0])
                    };

                    if flags.lacing != Lacing::NoLacing {
                        todo!("Support for lacing is not yet implemented!");
                    }

                    let data = file.read_range(position, end).await;

                    // read from keyframe to keyframe

                    // Data Payload - Frame Data

                    // https://github.com/Michael-A-Kuykendall/muxide#fragmented-mp4-dashhls

                }
            }
        }
    }
}