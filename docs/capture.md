# Windows capture and manual replay

## Pipeline

```text
visible cs2.exe window
  → Windows Graphics Capture / Direct3D surface
  → Windows Media Foundation H.264 hardware-acceleration path
  → independently finalized ~2 s MP4 segment
  → parsed sync-sample gate
  → hard-bounded rolling segment buffer
  → F8/manual replay or a Moment capture range
  → encoded-sample remux to final MP4
```

WGC timestamps come from the Windows system-relative clock. Frames remain Direct3D surfaces on the
normal path; OpenClip does not map each frame into CPU memory. Recording settings map Performance,
High, and Ultra to internal bitrate targets. Game resolution, 1080p, 1440p, and up to 60 FPS are
wired. The first supported hardware target is NVIDIA on Windows 10/11. OpenClip requires the NVIDIA
encoding driver DLL before starting and asks Media Foundation to enable hardware acceleration; it
does not silently select a deliberate software encoder or implement AMD/Intel backends yet.

## Audio

When game audio is enabled, WASAPI application loopback captures the CS2 process tree as 16-bit,
48 kHz stereo PCM. Media Foundation encodes it to AAC. This isolates CS2 more accurately than a
whole-system loopback but Windows/driver behavior can vary. Microphone and separate communications
audio mixing are deferred. Settings can already enumerate active Windows capture endpoints and
persist both the microphone enable flag and selected endpoint ID, so the future backend will not
require a settings migration. The UI explicitly identifies this as a saved preference, not active
microphone recording. If process loopback initialization fails, status and logs explicitly report
the limitation; the video replay continues.

## Crash and interruption safety

Every segment is a conventional MP4 which is fully closed before it becomes eligible for replay.
The current file uses `*.mp4.partial`. A crash can invalidate and lose that in-flight file (at most
roughly two seconds), but cannot make already indexed segments depend on its final `moov` box. On
startup, partial files are preserved for diagnostics and never indexed. Finalized segments are
parsed, sorted by modification time, and re-indexed with a continuous recovered timeline before new
segments are added. Tests cover startup recovery, a truncated neighbor beside two valid segments,
successful remux of the valid pair, and rejection of a segment whose first video sample is not a
sync sample.

MP4 was selected because the Windows encoder finalizes each short unit reliably and the Rust remuxer
can preserve H.264/AAC configuration and sample timing. If field testing exposes finalization defects,
the segment/container boundary is isolated so an MKV or fragmented-MP4 backend can replace it.

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

Segment rollover never closes or opens an encoder on the Windows Graphics Capture frame callback.
The next Media Foundation encoder is pre-warmed on a worker, and the previous independent MP4 is
flushed, parsed, and indexed by a serial finalizer worker. This prevents the 150–300 ms frozen-frame
runs that synchronous encoder startup/finalization caused at two-second boundaries. A resolution
generation gate prevents a late old-resolution job from entering a newly reset buffer. Ranged saves
wait on a condition variable until their newest required segment is finalized; they do not poll or
read an active `.partial` file.

Changing the clip storage folder affects newly saved manual and automatic output. It does not move
or redirect the active temporary rolling buffer.

## Diagnostics and validation

Structured native logs report target PID, resolution, FPS target/observed rate, bitrate, encoder
initialization/finalization time, dropped frame count, segment count/bytes, buffered seconds, remux
time, and errors. `nvidia-smi` can be used during a real recording session to verify an active NVIDIA
encoder session on the test machine.

Real validation requires CS2 (or a controlled capturable process during backend smoke testing), at
least 15 seconds of changing content, F8, and playback inspection for picture, duration, corruption,
and audio sync. Compile/tests alone must not be reported as successful gameplay capture.
