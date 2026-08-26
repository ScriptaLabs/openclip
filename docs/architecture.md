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

`CaptureEngine` exposes lifecycle/status without exposing frames to React. `openclip-capture` defines
encoded packet, hardware encoder, and audio track concepts. `ReplayBuffer` preserves an encoded
match-relative window. No raw frame or capture API crosses into the Tauri IPC layer.

The actual Windows capture, hardware encoders, audio, and segmented replay buffer are deferred until
Milestone 4. Their interfaces exist now to keep orchestration direction explicit, not to simulate
recording.

### Storage

SQLite is authoritative for metadata. The initial migration contains sessions, matches, rounds,
events, moments, moment-event links, clips, and settings. Foreign keys and delete behavior preserve
the graph. Video bytes are never stored in SQLite; `clips.path` points to a normal file.

`SqliteRepository` implements the core `MatchRepository` port. One GSI update is applied in a single
SQLite transaction so event graphs cannot be partially persisted. Repository tests round-trip a
complete match graph and settings through an in-memory SQLite database, while the CS2 listener test
posts real HTTP payloads into in-memory persistence.

### Tauri and React

The Tauri host is intentionally thin: it creates the application data directory, owns the database
connection, and exposes typed commands. The React application never performs capture.

In the desktop runtime, React pages consume an asynchronous `MatchRepository` implemented with Tauri
IPC. The host also exposes GSI/setup status and emits `gsi_update` plus match/round lifecycle events.
Typed fixtures implement the same interface only for the explicit browser development preview.

## Time and identity

UUID newtypes prevent accidentally mixing session, match, round, event, and moment IDs in Rust.
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
