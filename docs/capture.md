# Windows capture and manual replay

## Pipeline

```text
visible cs2.exe window
  → Windows Graphics Capture / Direct3D surface
  → persistent D3D11 registration / optional GPU scaling
  → continuous NVIDIA Encode API H.264 session
  → deterministic 30/60 FPS presentation timeline
  → forced-IDR independently finalized 2 s MP4 segment
  → parsed sync + actual H.264 IDR gate
  → hard-bounded rolling segment buffer
  → F8/manual replay or a Moment capture range
  → encoded-sample remux to final MP4
```

WGC timestamps come from the Windows system-relative clock. `FramePacer` converts irregular arrivals
into exact constant-frame-rate slots without sleeping on the callback. A late arrival may supply the
newest surface to more than one slot; diagnostics report those repeats explicitly. Frames remain
Direct3D textures: the capture pool's textures are registered with NVENC once and reused, while
configured 1080p/1440p conversion uses D3D11's video processor. OpenClip never maps frame pixels into
CPU memory on the normal path.

The NVIDIA driver DLL and API are loaded at runtime. OpenClip opens a DirectX NVENC session, uses the
P4 low-latency preset with VBR and no B-frames, and retains that session across every rolling segment.
Performance, High, and Ultra select bitrate targets. 
### Encoder selection

The `videoEncoder` setting picks the encoder: `auto` (default), `hardware` or `software`.
`auto` tries, in order:

1. NVIDIA NVENC (direct D3D11 path above) when `nvEncodeAPI64.dll` is present;
2. the GPU vendor's hardware Media Foundation H.264 encoder (Intel Quick Sync, AMD AMF and others),
   found with `MFTEnumEx`;
3. Windows' built-in software H.264 encoder.

`hardware` skips step 3 and reports an error when no hardware encoder starts. `software` uses only
step 3 and records at 30 FPS at most. The encoder in use is shown on the Overview status line, and
every clip records it.

The Media Foundation path (`mf_backend.rs`) copies each captured frame into a CPU-readable staging
texture, converts BGRA to BT.709 limited-range NV12 (`pixel.rs`), and feeds the transform. It is
more expensive than NVENC's zero-copy path (a 1080p frame is roughly 8 MB to read back and convert),
which is why NVENC stays first. Asynchronous (hardware) and synchronous (software) transforms are
both driven; segment boundaries follow IDR pictures, which are forced every two seconds. Output is
the same independently playable segments as NVENC, so saving still never re-encodes.

Hardware tests (`cargo test -p openclip-capture mf_backend`) encode synthetic pictures through every
encoder the machine offers. Only NVIDIA's transform has been exercised on real hardware so far;
Intel and AMD encoders follow the same Media Foundation contract but need field confirmation.

## Audio

When game audio is enabled, WASAPI application loopback captures the CS2 process tree as 16-bit,
48 kHz stereo PCM. Optional microphone capture opens the persisted endpoint ID (or the default input)
in the same format. If both sources are enabled, 10 ms chunks are paired and mixed with saturating
signed-16-bit addition. A bounded two-chunk jitter window absorbs small device-clock differences;
if one endpoint disappears, the surviving source continues instead of stalling the AAC track. Media
Foundation remains only as an AAC encoder; it never receives video. Audio failures are reported in
status and logs while video capture continues.

AAC encoders emit whole 1,024-sample access units. The segment muxer clips the final container sample
duration to the video boundary, so every two-second segment has exactly 96,000 audio ticks at 48 kHz.
Concatenating segments therefore does not accumulate A/V drift.

## Crash and interruption safety

Every segment is a conventional, independent MP4 written only after its complete encoded video unit
is available. The finalizer writes `*.mp4.partial`, parses it, verifies the first declared sync sample
contains a real IDR NAL, then atomically renames it. A crash can lose the in-memory video unit and
leave an AAC `.partial` (at most roughly two seconds), but cannot invalidate any prior segment. On
startup, partial files are preserved for diagnostics and never indexed. Finalized segments are
sorted, revalidated, and re-indexed with a continuous recovered timeline.

The hardware test forcibly terminated OpenClip mid-segment, restarted it, recovered 22 finalized
segments/44 seconds, and remuxed eight retained H.264/AAC segments into a decoded 960-frame file.
The in-flight partial was excluded. Unit coverage also rejects a P-slice falsely labeled as sync.

## Lifecycle and bounds

The supervisor waits for the largest visible window owned by `cs2.exe`, starts capture when found,
and returns to waiting after the window closes. It retries after relaunch or window recreation.
Minimization may temporarily stop capture because an invisible window is no longer a valid target.
Borderless/fullscreen-windowed and windowed modes are supported; fullscreen-exclusive capture is not
promised. A native resolution change finalizes the old segment and resets the buffer so incompatible
track configurations can never be remuxed together.

The rolling buffer is capped by configured duration and a conservative encoded-byte ceiling. Pruned
files are deleted only after leaving the index. Save holds a short pin on selected paths, writes a
new `.partial` output, parses it, and then renames it to the dated final path.

Segment rollover only asks the continuous encoder for an IDR; it never closes or opens NVENC on the
WGC callback. The next AAC-only encoder is pre-warmed, and the prior MP4 is muxed/validated by a
serial finalizer. This removes the former 150–300 ms Media Foundation video initialization freeze at
every boundary. Resolution and settings generation gates prevent late incompatible jobs from
entering a reset buffer. Ranged saves wait on a condition variable for their newest finalized
segment; they never poll or read an active partial.

Changing the clip storage folder affects newly saved manual and automatic output. It does not move
or redirect the active temporary rolling buffer.

## Diagnostics and validation

Structured native logs report target PID, resolution, target/source/encoded FPS, late-slot repeats,
early-callback skips, average/max NVENC time, bitrate, video/AAC initialization and finalization time,
encoded frame count, segment count/bytes, buffered seconds, remux time, and errors. `nvidia-smi` can
independently verify an active NVIDIA encoder session.

Real validation requires CS2 (or a controlled capturable process during backend smoke testing), at
least 15 seconds of changing content, F8, and playback inspection for picture, duration, corruption,
and audio sync. Compile/tests alone must not be reported as successful gameplay capture.
