//! Native harness for validating the remuxer without a browser.
//!
//! Usage: `cargo run -p ebml-wasm --example dump_segments -- <file.mkv> <out_dir> [track]`
//!
//! Writes `<out>/init_<track>.mp4` and `<out>/full_<track>.mp4` (init + first media
//! segment concatenated) for each muxable track, so `ffprobe`/`mp4box` can validate.

use ebml_wasm::ebml::Ebml;
use ebml_wasm::fs_source::FsSource;
use ebml_wasm::matroska_data::element_id_type_map;
use ebml_wasm::player::Demuxer;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = args.get(1).expect("usage: dump_segments <file.mkv> <out_dir> [track]");
    let out_dir = args.get(2).map(String::as_str).unwrap_or("/tmp/mkv_dump");
    let only_track: Option<u64> = args.get(3).and_then(|s| s.parse().ok());

    fs::create_dir_all(out_dir).unwrap();

    pollster::block_on(async {
        let source = FsSource::new(file);
        let ebml = Ebml::new(source, element_id_type_map());
        let demux = Demuxer::open(ebml).await;

        println!("timestamp_scale_ns = {}", demux.timestamp_scale());
        println!("duration_ms        = {}", demux.duration_ms());
        println!("tracks = {}", demux.tracks_json());

        // Parse the track JSON loosely to find track numbers + types.
        let tracks = demux.tracks_json();
        for entry in tracks.trim_matches(['[', ']']).split("},{") {
            let number = field(entry, "\"number\":").and_then(|s| s.parse::<u64>().ok());
            let Some(number) = number else { continue };
            if let Some(t) = only_track {
                if t != number {
                    continue;
                }
            }
            let kind = field_str(entry, "\"type\":\"");
            let mime = field_str(entry, "\"mime\":\"");
            if mime.is_empty() {
                println!("track {number} ({kind}): no MP4 mime — skipping");
                continue;
            }

            let Some(init) = demux.init_segment(number) else {
                println!("track {number}: no init segment");
                continue;
            };
            let media = demux.media_segment(number, 0, 4000).await;

            let init_path = format!("{}/init_{}.mp4", out_dir, number);
            fs::write(&init_path, &init).unwrap();

            match media {
                Some(media) => {
                    let mut full = init.clone();
                    full.extend_from_slice(&media);
                    let full_path = format!("{}/full_{}.mp4", out_dir, number);
                    fs::write(&full_path, &full).unwrap();
                    println!(
                        "track {number} ({kind}, {mime}): init {} B, media {} B → {}",
                        init.len(),
                        media.len(),
                        Path::new(&full_path).display()
                    );
                }
                None => println!("track {number}: no media segment in [0,4000ms]"),
            }

            // Mid-file segment: exercises the seek-target path with a non-zero tfdt.
            if demux.duration_ms() > 606_000 {
                if let Some(media) = demux.media_segment(number, 600_000, 604_000).await {
                    let mut full = init.clone();
                    full.extend_from_slice(&media);
                    let mid_path = format!("{}/mid_{}.mp4", out_dir, number);
                    fs::write(&mid_path, &full).unwrap();
                    println!("track {number}: mid-file [600s,604s] media {} B → {}", media.len(), mid_path);
                }
            }
        }
    });
}

fn field<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
    let start = entry.find(key)? + key.len();
    let rest = &entry[start..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(&rest[..end])
}

fn field_str(entry: &str, key: &str) -> String {
    match field(entry, key) {
        Some(v) => v.trim_matches('"').to_string(),
        None => String::new(),
    }
}
