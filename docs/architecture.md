# Architecture

OpenClip separates game understanding, recording, metadata, and presentation so each can evolve
without pulling platform or game-specific details across the entire application.

## Dependency direction

```text
                         openclip-core
                      (domain types + ports)
                         ▲    ▲    ▲
                         │    │    │
             openclip-cs2│    │    │openclip-capture
                         │    │    │
                         │ openclip-buffer
                         │    │
                    openclip-storage
                         ▲
                         │
                    Tauri host ─── React UI
```

`openclip-core` has no dependency on Tauri, SQLite, Windows APIs, or CS2 payload shapes. All other
native components depend inward on its vocabulary. This prevents a raw GSI field or GPU texture
from becoming a UI or database concern.

## Match-centric domain

The authoritative relationship is:

```text
GameSession
└── Match
    ├── Round
    │   └── GameEvent
    └── Moment
        ├── GameEvent references
        └── optional Clip file

Clip (manual replay)
└── optional Match reference
```

A `GameEvent` is game-agnostic and carries optional JSON metadata for facts such as weapon names.
The CS2 adapter translates state changes into these events. The core UI and moment engine do not
need to know the original GSI JSON layout.

A `Moment` is a logical time window rather than a synonym for a video file. Nearby qualifying events
extend one window, which avoids overlapping files for a quick multi-kill. `clip_path` is optional so
the event and intended window can survive capture or extraction failure and be retried.

## Boundaries

### Game adapters

`GameAdapter::process_state` accepts native bytes and emits a `GameStateUpdate` containing the
sessions, matches, rounds, and generic events changed by one snapshot. CS2 GSI installation, HTTP
handling, parsing, and state differencing stay in `openclip-cs2`. That crate does not depend on
capture code. The listener binds an IPv4 loopback address rather than an externally reachable
interface.

### Match and moment management

`MatchManager` owns session, match, and round lifecycle. `MomentDetector` independently consumes
events and groups qualifying ones. `WindowedMomentDetector` is the first implementation and uses
saturating millisecond arithmetic to make pre-roll safe near match start.

### Capture and replay buffer

`RecorderService` supervises the CS2 window lifecycle. Windows Graphics Capture delivers D3D11
textures with system-relative monotonic timestamps. A narrowly scoped platform module dynamically
loads NVIDIA's Encode API, registers frame-pool textures once, and feeds one continuous NVENC H.264
session without mapping pixels into CPU memory. Other GPUs use a Media Foundation H.264 encoder
(hardware, then Windows' CPU encoder) behind the same segment interface. A deterministic pacer maps irregular WGC arrivals
onto exact 30/60 FPS presentation slots. D3D11's video processor handles configured output scaling
on the GPU.

CS2 process-tree WASAPI loopback and an optional selected microphone endpoint supply 48 kHz stereo
PCM. When both are enabled they are mixed with saturating signed-16-bit arithmetic before a separate
Media Foundation AAC-only worker. Video no longer passes through Media Foundation.

NVENC is not restarted at segment boundaries. Every 120 frames at 60 FPS (or 60 at 30 FPS), OpenClip
sets `FORCEIDR | OUTPUT_SPSPPS`, converts returned Annex-B access units to MP4 samples, and pairs them
with AAC. Audio's final sample duration is clipped to the exact video clock, preventing per-segment
drift from accumulating. The serial finalizer writes a conventional `.mp4.partial`, parses it,
requires both container sync metadata and an actual H.264 IDR NAL, atomically renames it, and only
then indexes it. Previously finalized MP4s therefore survive abrupt termination; only the
in-memory/in-flight unit is lost. The buffer exposes both a
recent-window selector for F8 and a logical-range selector for automatic moments; a condition
variable synchronizes ranges whose newest segment is still active.
The buffer has both duration and byte ceilings. Saving selects a keyframe-safe recent window and
uses `mp4io` to copy encoded H.264/AAC samples into a new MP4—there is no video decode or re-encode.
On startup, finalized segments are validated and re-indexed while any interrupted `.partial` is
kept only for diagnostics. Resolution changes finalize the old segment and reset the incompatible
rolling window. Persisted capture-setting changes advance a generation clock and restart the active
session, so FPS, quality, resolution, and audio-device changes cannot partially reconfigure a stream.

### Storage

SQLite is authoritative for metadata. Migration 0002 adds `captured_clips`; migration 0003 adds
clip-origin relationships without rewriting legacy tables. Manual replays do not require or invent
a moment. Automatic clip and Moment metadata are committed together when capture succeeds, while a
logical Moment can still persist without a clip after a capture failure. Video bytes are never
stored in SQLite.

`SqliteRepository` implements the core `MatchRepository` port. One GSI update is applied in a single
SQLite transaction so event graphs cannot be partially persisted. Repository tests round-trip a
complete match graph and settings through an in-memory SQLite database, while the CS2 listener test
posts real HTTP payloads into in-memory persistence.

### Tauri and React

The Tauri host creates the application data directory, owns the database and recorder, registers F8,
and exposes typed commands. An `AutomaticMomentService` observes persisted generic GSI updates,
runs the moment timer, and delegates ranged saves to the recorder without teaching capture code what
a kill is. F8 and `save_replay` call the same independent manual recorder operation. The React
application never performs capture and does not need to remain focused. Recording controls persist
quality, resolution, FPS, game-audio, and microphone preferences; endpoint enumeration and live
capture/mixing use the same persisted microphone device ID.

In the desktop runtime, React pages consume an asynchronous `MatchRepository` implemented with Tauri
IPC. The host also exposes GSI/setup status and emits `gsi_update` plus match/round lifecycle events.
Typed fixtures implement the same interface only for the explicit browser development preview.
Native commands validate/persist storage directories, open only database-resolved clip paths, and
copy qualifying match moments to deterministic, collision-safe export names.

## Time and identity

UUID newtypes prevent accidentally mixing session, match, round, event, moment, and clip IDs in Rust.
Absolute timestamps are UTC `DateTime` values. Match-relative video windows use unsigned
milliseconds; this makes persistence and IPC representations predictable and avoids floating-point
timestamp drift.

## Reliability rules

- production paths return structured errors rather than panicking;
- SQLite foreign keys are enabled and metadata writes are idempotent upserts;
- moment/event associations update transactionally;
- capture failures must not delete relevant replay segments or metadata;
- logs are structured at the native boundary;
- encoded media stays outside SQLite and should use crash-tolerant segmented containers.
