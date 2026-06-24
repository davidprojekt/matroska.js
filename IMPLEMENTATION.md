# Implementing an in-browser MKV player (MKV → fMP4 → MSE)

A from-scratch guide to everything required to play a Matroska (`.mkv`/`.webm`) file
directly in a browser by **remuxing** (not transcoding) it into fragmented MP4 and
feeding that to a `<video>` element through Media Source Extensions (MSE).

This is the distilled knowledge — including the non-obvious traps — behind this
repository. It assumes you can read bytes and write bytes; everything else is here.

---

## 0. The big picture

```
URL ──HTTP range reads──▶ EBML parser ──▶ Matroska model (tracks, cues, clusters)
                                              │
                       per track:  CodecPrivate ──▶ MSE codec string + MP4 config box
                                              │
   time range request ──▶ collect cluster frames ──▶ fMP4 init + media segments
                                              │
                                   MediaSource / SourceBuffer.appendBuffer
                                              │
                                       <video> + player UI
```

You are doing **container translation only**. The encoded video/audio frames are
copied byte-for-byte; you only rewrite the wrapping (EBML → ISO-BMFF) and reconstruct
the timing metadata MP4 needs but Matroska doesn't store. Whether it then *plays*
depends entirely on whether the browser can decode the codec (see §6).

Pipeline stages, each a section below:

1. **Byte source** — HTTP range reads, CORS, caching, speculative prefetch.
2. **EBML parsing** — the generic element grammar.
3. **Matroska model** — which elements matter and their semantics (incl. the
   Segment content-start offset, the seek index, and block/lacing parsing).
4. **Codec mapping** — Matroska CodecID → MSE MIME + MP4 config boxes.
5. **fMP4 muxing** — the exact box tree, and the timing reconstruction (B-frames!).
6. **Segmentation** — how to cut the timeline into appendable segments.
7. **MSE playback** — the SourceBuffer feed loop, seeking, audio switching, subtitles.
8. **Player UI** — wrapping it in video.js v10.
9. **Gotchas** — the bugs that will bite you, with the fix.
10. **Validation** — how to prove your output is correct without a browser.

---

## 1. The byte source (HTTP range reads)

You never download the whole file. You read **byte ranges** on demand.

### Requirements on the server
- Must honor `Range: bytes=START-END` and reply **`206 Partial Content`** with the
  requested slice. A server that replies `200` with the whole body (some
  `python -m http.server` setups) **will not work** — detect this and surface a clear
  error rather than silently treating it as empty.
- If the media is served from a **different origin** than the page, it needs CORS:
  `Access-Control-Allow-Origin`. To read the `Content-Range` response header
  cross-origin (needed for suffix ranges, below) the server must also send
  `Access-Control-Expose-Headers: Content-Range` — most don't, so don't depend on it.

### A read cache
Naive parsing makes thousands of tiny reads (a variable-length integer is read one
byte at a time). Without batching, that's thousands of HTTP requests. Cache reads in
**aligned blocks** (e.g. 16 KB): a read fetches the block(s) covering it once;
adjacent reads hit cache. Keep the cache bounded (LRU); 16 KB × a few thousand
entries is plenty. Too small a block (1 KB) = request storm; too big = wasted
bandwidth on scattered reads.

### Speculative cold-start prefetch
On first access, fire **two requests in parallel before parsing anything**:
- the **first ~32 KB** (`bytes=0-32767`) — holds the EBML header, Info, Tracks;
- the **last ~256 KB** (`bytes=-262144`, a *suffix range*) — usually holds the Cues
  and a SeekHead in streamable files.

Seed both into the cache so the initial parse and the trailing seek-index read are
warm. **Trap:** the suffix range's absolute offset is only known from the
`Content-Range` header. If that header isn't readable (cross-origin, not exposed),
you do **not** know where those bytes belong — seeding them at offset 0 corrupts the
header region. Only seed the tail when its offset is actually known; the head is
always at 0 so it's always safe.

---

## 2. EBML parsing

Matroska is EBML: a tree of elements, each `[ID][Size][Data]`.

- **Element ID** — a variable-length integer. The number of leading zero bits in the
  first byte gives the total length (0 leading zeros → 1 byte, etc.). For IDs you keep
  the length-marker bits (the ID *is* the raw bytes, e.g. Segment = `0x18538067`).
- **Element Size** — same variable-length encoding, but you **strip** the marker bit
  to get the value (the data length in bytes). A size of all-1s means "unknown size"
  (used for live streams / some clusters) — handle by reading until the parent ends or
  a known sibling appears.
- **Data** — interpreted per the element's declared type: unsigned int, signed int,
  float (4 or 8 bytes — **never zero-pad a float to widen it**, that changes the
  value), UTF-8 string, date, binary, or *master* (a nested list of elements).

Build a **lazy iterator**: `next()` reads one element's ID+Size, and for master
elements returns a sub-iterator positioned at the content (so you can skip whole
subtrees — e.g. skip a multi-MB Cluster by jumping `Size` bytes). For binary
payloads, return the **(start, end) byte range**, not the bytes — you read frame
bytes only when you actually mux them.

You need a map from element ID → type. The official Matroska schema XML lists every
element; generate the ID/type table from it (a build-time codegen step or a checked-in
table).

---

## 3. The Matroska model you must extract

Top level: an `EBML` header element, then a **`Segment`** master containing
everything else.

### 3.1 The Segment content-start offset — the anchor for everything
Record the absolute file offset where the **Segment master's *content*** begins (i.e.
just after the Segment's ID+Size header). **Every** `SeekPosition` (in SeekHead) and
**every** `CueClusterPosition` (in Cues) is measured **relative to this offset**.
Get this wrong and all seeking lands in the wrong place. (Store it once at open.)

### 3.2 Info — timing scale and duration
Inside `\Segment\Info`:
- **`TimestampScale`** (default `1_000_000` ns = 1 ms). All block/cluster timestamps
  in the file are in units of this many nanoseconds. With the default, "ticks" = ms.
- **`Duration`** — a float, in TimestampScale units. `duration_seconds = Duration *
  TimestampScale / 1e9`.

### 3.3 Tracks — what's in the file
`\Segment\Tracks\TrackEntry`, one per track. Fields you need:
- `TrackNumber` (used as the MP4 track id and in block headers), `TrackType`
  (1=video, 2=audio, 17=subtitle, …), `CodecID` (e.g. `V_MPEG4/ISO/AVC`),
  `CodecPrivate` (the codec's setup data — becomes the MP4 config box, see §4),
  `DefaultDuration` (nanoseconds per frame — needed for sample durations),
  `Language`/`LanguageBCP47`, `FlagDefault`, `FlagForced`.
- **Video** sub-master (`\TrackEntry\Video`): `PixelWidth`, `PixelHeight`,
  `DisplayWidth/Height`. *These are nested inside the Video element — a common bug is
  reading them as direct children of TrackEntry, where they don't exist.*
- **Audio** sub-master (`\TrackEntry\Audio`): `SamplingFrequency` (float),
  `Channels`, `BitDepth`.

### 3.4 SeekHead and Cues — finding things and seeking
- **`SeekHead`** is an index of **where top-level elements live** (Tracks, Info, Cues,
  …) as `(SeekID, SeekPosition)` pairs, position relative to Segment content-start. It
  is **not** a time index — it only locates elements. Use it to jump straight to Cues
  (often at end of file) without scanning clusters.
- **`Cues`** *is* the time→byte seek index: `CuePoint`s of `CueTime` (in TimestampScale
  units) + `CueTrackPositions` (`CueTrack`, `CueClusterPosition` relative to Segment
  content-start, optional `CueRelativePosition`). To seek to time *t*: find the cue
  with the greatest `CueTime ≤ t` and jump to `SegmentContentStart + CueClusterPosition`.
- **Trap:** many files index cues for **every track**, including frequent
  *mid-cluster* audio cues — you can get thousands of them. For segment boundaries
  (§6) filter to the **video track's** cues and **deduplicate**; those are the real
  keyframe/cluster boundaries.
- **Fallback:** if there are no Cues, you must linearly scan clusters and build your
  own time→offset index (degraded; log it).

### 3.5 Clusters and blocks — the actual media
`\Segment\Cluster` holds a `Timestamp` (uint, cluster's base time in TimestampScale
units) followed by blocks:
- **`SimpleBlock`** (binary) — the common case. Layout of its payload:
  1. **Track number** — variable-length integer.
  2. **Relative timecode** — `int16`, signed, big-endian, **relative to the cluster
     Timestamp**. Absolute PTS (ticks) = `cluster_timestamp + relative_timecode`.
  3. **Flags** — 1 byte: bit `0x80` = keyframe; bits `0x06` = lacing
     (`00`=none, `01`=Xiph, `11`=EBML, `10`=fixed); bit `0x08` = invisible;
     `0x01` = discardable.
  4. **Frame data** (one frame, or several if laced).
- **`BlockGroup`** master — wraps a `Block` (same layout as SimpleBlock but the
  keyframe flag is reserved) plus metadata. Keyframe-ness is determined by the
  **absence of a `ReferenceBlock`** child. May carry `BlockDuration` (used for
  subtitle cue end times).

### 3.6 Lacing — required, not optional
Audio is routinely **laced**: one block carries several frames (e.g. multiple
1024-sample AAC frames). You must unpack all three lacing types or audio breaks:
- **Fixed**: header byte = `frame_count − 1`; remaining bytes split equally.
- **Xiph**: `frame_count − 1` sizes, each a run of `0xFF` bytes summed plus a final
  `<0xFF` byte; the last frame takes the remainder.
- **EBML**: first size is an unsigned vint; subsequent sizes are **signed vint
  deltas** from the previous; the last frame takes the remainder.

---

## 4. Codec mapping (Matroska → MSE)

For each track derive (a) the MSE codec string for
`MediaSource.isTypeSupported("video/mp4; codecs=\"…\"")` and (b) the MP4 config box
built from `CodecPrivate`. Gate every track with `isTypeSupported` and surface
unsupported ones — remuxing always works, but the *browser* may not decode the codec
(e.g. HEVC in Firefox, AC-3 in Chrome/Firefox).

| Matroska CodecID    | MP4 sample entry | Codec string                       | Config box ← CodecPrivate |
|---------------------|------------------|------------------------------------|---------------------------|
| `V_MPEG4/ISO/AVC`   | `avc1`           | `avc1.PPCCLL` (hex of avcC[1..=3])  | `avcC` ← CodecPrivate **verbatim** |
| `V_MPEGH/ISO/HEVC`  | `hvc1`           | `hvc1.…` (parse hvcC, see below)    | `hvcC` ← CodecPrivate **verbatim** |
| `V_VP9`             | `vp09`           | `vp09.PP.LL.DD` (often defaulted)   | `vpcC` (built; CodecPrivate rare) |
| `V_AV1`             | `av01`           | `av01.P.LLT.DD` (parse av1C)        | `av1C` ← CodecPrivate **verbatim** |
| `A_AAC`             | `mp4a`           | `mp4a.40.N` (N = AudioObjectType)   | `esds` (built from AudioSpecificConfig) |
| `A_OPUS`            | `Opus`           | `opus`                             | `dOps` (built from OpusHead) |
| `A_AC3`             | `ac-3`           | `ac-3`                             | `dac3` |
| `A_EAC3`            | `ec-3`           | `ec-3`                             | `dec3` |
| `S_TEXT/UTF8`       | (text track)     | — (SubRip → WebVTT, §7)            | — |
| `S_TEXT/WEBVTT`     | (text track)     | — (pass through)                   | — |
| `S_TEXT/ASS`,`SSA`  | (text track)     | — (needs libass; out of scope)     | — |

Key simplifying facts:
- For **H.264/HEVC/AV1**, the Matroska `CodecPrivate` *is already* the
  `AVCDecoderConfigurationRecord` / `HEVCDecoderConfigurationRecord` /
  `AV1CodecConfigurationRecord` — copy it verbatim as the box payload.
- **H.264** stores frames as **length-prefixed NAL units** (`nal_length_size` is in
  avcC, almost always 4). MP4 `mdat` wants exactly that — so frame bytes copy straight
  through, **no Annex-B conversion**. (HEVC likewise.)
- **`avc1.PPCCLL`** is literally the hex of avcC bytes 1, 2, 3 (profile, compat,
  level), e.g. High@L4.0 → `avc1.640028`.
- **AAC** `CodecPrivate` is the 2-byte AudioSpecificConfig; the AudioObjectType is its
  top 5 bits (LC = 2 → `mp4a.40.2`). The `esds` box wraps it in MPEG-4 descriptors
  (ES_Descriptor → DecoderConfigDescriptor → DecoderSpecificInfo with the ASC).
- **`hvc1` codec string** is fiddly: parse the hvcC for profile-space/tier/profile-idc
  (byte 1), the 32-bit compatibility flags (emitted bit-reversed), the level (byte 12),
  and the constraint bytes; format `hvc1.{space}{idc}.{compatHex}.{tier}{level}.{constraints}`.

---

## 5. fMP4 muxing (ISO-BMFF)

Fragmented MP4 splits into:
- an **initialization segment**: `ftyp` + `moov` (track definitions, no samples);
- **media segments**: `moof` (timing/metadata) + `mdat` (frame bytes), one or more.

Per track you append the init segment **once**, then media segments. (For multiple
tracks use one SourceBuffer per track; init each separately — see §7.)

### 5.1 Init segment box tree
```
ftyp                       major brand iso5/iso6/mp41
moov
 ├─ mvhd                   timescale, duration, next-track-id
 ├─ trak
 │   ├─ tkhd               track id, (width<<16, height<<16) for video, volume for audio
 │   └─ mdia
 │       ├─ mdhd           media timescale (see 5.3), language (packed ISO-639)
 │       ├─ hdlr           'vide' or 'soun'
 │       └─ minf
 │           ├─ vmhd | smhd
 │           ├─ dinf/dref  one self-contained 'url ' entry
 │           └─ stbl
 │               ├─ stsd   one sample entry:
 │               │          video: VisualSampleEntry(w,h,…)+avcC/hvcC/av1C/vpcC
 │               │          audio: AudioSampleEntry(ch,rate)+esds/dOps/dac3
 │               └─ stts/stsc/stsz/stco   all EMPTY (zero entries) — data lives in moof
 └─ mvex
     └─ trex               per track: default sample desc index = 1, defaults 0
```
`mvex`/`trex` are **mandatory** for fragmented files — omit them and nothing plays.

### 5.2 Media segment box tree
```
moof
 ├─ mfhd                   sequence_number (increasing)
 └─ traf
     ├─ tfhd               track id, flags 0x020000 (default-base-is-moof)
     ├─ tfdt               version 1, 64-bit baseMediaDecodeTime  ← ABSOLUTE decode time
     └─ trun               version 1 (signed composition offsets); per sample:
                            duration, size, flags, composition_time_offset (PTS−DTS)
mdat                       all sample data, concatenated, in the trun's order
```
- `trun` flags set: data-offset (`0x1`), sample-duration (`0x100`), sample-size
  (`0x200`), sample-flags (`0x400`), sample-composition-time-offset (`0x800`).
- **data offset**: build the `moof` once to learn its size, then rewrite `trun`'s
  `data_offset = moof_size + 8` (the start of mdat's payload). Box sizes don't depend
  on the offset value, so a two-pass build is exact.
- **sample flags**: keyframe = `0x02000000` (depends-on=2, not a non-sync sample);
  non-keyframe = `0x01010000` (depends-on=1, is-non-sync-sample). Audio: all keyframes.
- **`tfdt` must carry the *absolute* decode time** of the segment's first sample. This
  is what makes segments position-independent: seeking = "append the segment whose
  tfdt is the seek time," no `SourceBuffer.timestampOffset` juggling.

### 5.3 Timescales
- **Video**: use timescale = `1e9 / TimestampScale` (e.g. 1000 = ms). Then Matroska
  ticks map 1:1 to MP4 time units.
- **Audio**: use timescale = **sample rate** (e.g. 48000). Each AAC frame's duration is
  then exactly `1024` (samples per frame; Opus 960, AC-3 1536), so a segment accrues
  no rounding drift. `baseMediaDecodeTime = round(first_frame_ticks · TimestampScale ·
  sample_rate / 1e9)` (i.e. ms → samples).

### 5.4 THE hard part: reconstructing DTS and composition offsets for B-frames
Matroska stores **only PTS**, in **decode order**. MP4 `trun` needs a monotonic
**DTS** plus a per-sample **composition offset = PTS − DTS**. If you naively set
DTS = PTS on a stream with B-frames, DTS is non-monotonic → the fragment is invalid /
MSE rejects it.

Use the **"DTS = sorted PTS"** reconstruction, per segment:
1. Take the segment's samples in storage = decode order; collect their PTS.
2. `sorted = PTS sorted ascending`. Assign `DTS[i] = sorted[i]` to the *i*-th
   decode-order sample. DTS is now monotonic by construction.
3. `duration[i] = sorted[i+1] − sorted[i]` (exact — **no drift**, even at ms
   timescale); the last sample uses `DefaultDuration`.
4. `composition_offset[i] = PTS[i] − DTS[i]` (may be negative → that's why `trun` is
   version 1 / signed).
5. `baseMediaDecodeTime = sorted[0]` (= min PTS in the segment).

This is provably correct for any reordering: presentation time `DTS + offset` always
equals the original PTS, and durations sum exactly to the presentation span.

Audio is never reordered: composition offset 0, durations as in §5.3.

---

## 6. Segmentation — cutting the timeline

You serve the player segments on request: "give me track *T* from time *a* to *b*."

**Tile on whole clusters, never mid-cluster.** A cluster is a complete,
keyframe-bounded set of GOPs. If you cut by per-frame PTS you *will* drop reordered
B-frames near the boundary (a frame with PTS < end can appear, in decode order, after
one with PTS ≥ end), leaving **holes in the presentation timeline** that stall the
decoder even though the buffer "looks" full. Instead:

- Start at the cluster found via the Cues for time *a*.
- Include each cluster **whole**; peek its `Timestamp` (a tiny leading read) and
  **stop at the first cluster whose timestamp ≥ b**.
- Read each included cluster's bytes in **one request** and parse blocks from memory —
  not a request per block (that's the difference between "playable" and a request
  storm that can't keep up).

**Boundaries** come from the **video keyframe cues** (§3.4) — deduplicated, ~one per
GOP. Tiling on raw all-track cues gives thousands of tiny overlapping segments and
constant redundant fetching. The player tiles `[cue[i], cue[i+1])`; because each cue is
a cluster/keyframe boundary the segments abut exactly. Overlap (if cues are denser than
clusters) is harmless — MSE overwrites; **gaps** are what stall you, so prefer
over-inclusion to truncation.

---

## 7. MSE playback

Standard `HTMLMediaElement` MSE — independent of any player library.

```
const ms = new MediaSource();
video.src = URL.createObjectURL(ms);
await sourceopen;
ms.duration = durationSeconds;          // so the seekbar reflects total length
// per track you will play:
if (!MediaSource.isTypeSupported(mime)) { /* surface unsupported */ }
const sb = ms.addSourceBuffer(mime);    // one for video, one for audio
sb.appendBuffer(initSegment);           // ONCE per buffer
// then media segments, pumped on 'updateend'
```

### The feed loop
- **Never** call `appendBuffer`/`remove` while `sb.updating` is true. Maintain a queue
  per SourceBuffer and pump it on each `updateend`.
- On `timeupdate`/`waiting`, **top up** each buffer so it stays ~20 s ahead of
  `currentTime`. Crucial: measure "ahead" as the end of the **buffered range that
  contains the playhead**, not the global last range — otherwise after a seek into an
  un-buffered area you compute "lots buffered" (from the old range) and never refill →
  permanent stall. If the playhead isn't inside any buffered range, treat buffered-end
  as the current time so a fill is triggered.
- Call `mediaSource.endOfStream()` only when **all** tracks are fully fed and idle.

### Seeking
On the `seeking` event: abort in-flight appends, clear the queue, reset each track's
feed pointer to the segment for the new time, and append from there. **Do not**
re-append the init segment. Because `tfdt` is absolute, the appended segments line up
with `currentTime` automatically. Guard against a stale in-flight segment landing after
the seek (a generation counter you check after each `await`).

### Switchable audio (different codecs)
MSE plays only up to `min(videoBufferedEnd, audioBufferedEnd)`, so keep both fed. To
switch audio track: drain the audio queue, wait for idle, `sb.changeType(newMime)` if
the codec differs, `sb.remove(0, Infinity)`, then re-feed the new track's init +
segments from the current playhead. (video.js v10 has **no** audio-track feature — you
build this menu yourself; see §8.)

### Subtitles
Convert each text track to **WebVTT** and add it as a native text track
(`<track>` with a Blob URL, or `addTextTrack` + `VTTCue`); the player's captions menu
picks them up. SubRip→WebVTT is tiny: prepend `WEBVTT\n\n` and use `.` instead of `,`
in cue timestamps; `S_TEXT/WEBVTT` passes through.
- **Cost trap:** subtitle cues are scattered across **all** clusters, with no index, so
  extracting one subtitle track means scanning the whole file. Do it **lazily** (only
  when the user selects that track), read clusters in bulk, and never at startup for
  every track (that's a multi-file-scan request storm). ASS/SSA need libass — list them
  but don't try to render.

---

## 8. Player UI (video.js v10 = `@videojs/html`)

v10 is a ground-up rewrite: ESM-only **web components**, not the v8 `videojs()`
factory. Minimal embed:

```html
<script type="module">import '@videojs/html/video/skin';</script>  <!-- + its CSS -->
<video-player>
  <video-skin>
    <video slot="media" playsinline></video>   <!-- the real element you drive -->
  </video-skin>
</video-player>
```
- `import '@videojs/html/video/skin'` registers `<video-player>`, `<video-skin>` and
  all the control elements; also import `@videojs/html/video/skin.css`.
- The provider auto-discovers the inner `<video>` via `querySelector`. **You** own the
  MSE pipeline: grab that element and drive `MediaSource`/`SourceBuffer` yourself (§7).
  v10 just renders the UI shell + captions menu on top of the native element.
- It reads native `video.textTracks`, so WebVTT subtitle tracks appear in its captions
  menu automatically.
- It has **no audio-track feature** — render your own `<select>`/menu and call your
  audio-switch routine.
- Bundle with Vite. Keep the wasm package out of esbuild dep-prebundling so the wasm
  `new URL(..., import.meta.url)` reference survives and the `.wasm` is emitted as an
  asset.

---

## 9. The gotchas, collected

Every one of these cost real debugging time:

1. **Float widening** — zero-padding a 4-byte EBML float to 8 bytes changes its value.
   Decode 4-byte and 8-byte floats with their native widths.
2. **Nested track dimensions** — `PixelWidth`/`Channels`/etc. live inside the
   `Video`/`Audio` sub-masters, not directly under `TrackEntry`.
3. **Suffix-range prefetch clobber** — if you can't read `Content-Range` (cross-origin,
   header not exposed), you don't know where the tail bytes go; seeding them at offset 0
   overwrites the header and the file "parses" to zero tracks. Only seed when the offset
   is known.
4. **Server returns 200, not 206** — silently yields empty reads → empty everything.
   Preflight a Range request and report it.
5. **Request storms** — 1-byte reads and per-block reads over the network are death.
   Cache in large blocks; read whole elements/clusters in one request and parse from
   memory.
6. **B-frame DTS** — the single biggest correctness item. Decode-order PTS only; you
   must reconstruct monotonic DTS and signed composition offsets (§5.4). Wrong → invalid
   fragments or stalls.
7. **Mid-cluster cutting drops B-frames** — tile on whole clusters (§6), or you get a
   small hole at every segment boundary that stalls the decoder while the buffer looks
   full.
8. **All-track cue spam** — files index cues for every track; thousands of dense,
   duplicated, mid-cluster boundaries. Filter to the video track and dedupe (§3.4/§6).
9. **Lacing isn't optional** — AAC is commonly laced; unhandled → broken audio (§3.6).
10. **`bufferedEnd` at the playhead, not globally** — else backward seeks into a gap
    stall forever (§7).
11. **`mvex`/`trex` missing** — fragmented MP4 won't play without them.
12. **Codec support ≠ remux success** — `isTypeSupported`-gate every track; HEVC
    (Firefox), AC-3 (Chrome/Firefox) remux fine but won't decode there.
13. **Audio timescale** — use sample-rate so frame durations are exact integers; ms
    timescale for audio drifts.

---

## 10. Validating without a browser

You can prove the muxer correct natively:

- Drive the demuxer with a local-file byte source and dump, per track, the **init
  segment** + a **media segment**; concatenate and run `ffprobe`/`mp4box`. Check the
  codec, packet count, and PTS span.
- **Mid-file** segment (e.g. [600s, 604s]) to exercise the seek path: confirm `tfdt`
  carries the right absolute time and it decodes.
- **Stitch** many consecutive segments into one file and:
  - `ffmpeg -v error -i stitched.mp4 -f null -` → must decode with **zero errors**
    (ignore the first frames of a mid-file stitch — cold-start lacks references);
  - scan packet DTS deltas → the **max gap must be ~one frame**; any larger gap is a
    timeline hole that will stall playback.

This is exactly how the boundary-hole and cue-spam bugs above were caught and fixed.

---

## Out of scope (here) / next steps

- **ASS/SSA subtitle rendering** via [libass](https://github.com/libass/libass)
  (compiled to WASM) — listed but not rendered today.
- **VP9/AV1 codec strings** are best-effort defaults unless parsed fully from the
  bitstream/config record.
- **AC-3 `dac3`** is best-effort; full correctness needs parsing the bitstream
  syncframe.
- A continuous global audio sample-timeline (vs. per-segment ms→sample rounding) if you
  observe sub-millisecond audio seam artifacts.
