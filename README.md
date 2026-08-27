# OpenClip

OpenClip is a lightweight, local-first automatic game clipping application. It is being built for
Counter-Strike 2 on Windows 10/11 with no accounts, cloud dependency, advertisements, social feed,
or telemetry by default.

OpenClip is **match-centric**. Automatic moments belong to rounds and matches; manual replays are
separate clip records which can optionally reference the active GSI match.

## Current milestone

Milestone 5 replaces the video transcoder with a direct NVIDIA NVENC pipeline built for stable,
edit-ready replay timing while retaining Milestone 4 automatic highlights:

- Windows Graphics Capture targeting the largest visible `cs2.exe` window
- WGC D3D11 textures registered directly with a dynamically loaded NVIDIA Encode API session;
  there is no per-frame CPU pixel readback or Media Foundation video encoder
- one continuous NVENC session with deterministic 30/60 FPS pacing, persistent texture
  registrations, low-latency P4 encoding, and GPU video-processor scaling
- process-tree WASAPI game audio plus optional selected-device microphone capture/mixing, encoded as
  AAC at 48 kHz stereo
- independently finalized two-second MP4 segments with exact video/audio clocks; every segment is
  verified to contain a real H.264 IDR rather than trusting container sync metadata
- hard duration and byte bounds, keyframe-safe selection, and encoded-sample MP4 remuxing on save
- F8 global Save Replay plus the shared `save_replay` Tauri command
- event-driven moments with configurable pre/post-roll, same-round multi-kill merging, deterministic
  importance scoring, and delayed finalization
- ranged, keyframe-safe remuxing into automatic clips linked to their Moment, Round, and Match
- separate persistent manual and automatic clip sources; F8 remains independent
- real match-detail highlights/timeline markers, OS playback, and per-match interesting-moment export
- native validated storage selection used by future manual and automatic clips
- launch/close/relaunch monitoring, live-settings restart, resolution-change reset, crash recovery,
  and source/encoded cadence plus encode-latency diagnostics
- persistent quality, resolution, frame-rate, game-audio, and microphone device preferences
- mint/blue/purple/neutral accents and optional restrained clip-captured feedback
- the supplied banner as the sole sidebar brand and the supplied monochrome app icon

## Architecture

```text
React UI ── typed Tauri IPC ── application state
                                  │
              ┌───────────────────┼────────────────────┐
              │                   │                    │
        CS2 GameAdapter      match/moment logic   capture + replay ports
              │                   │                    │
              └──── generic GameEvent / domain types ──┘
                                  │
                           SQLite repository
```

The Rust crates are deliberately separated:

| Crate | Responsibility |
| --- | --- |
| `openclip-core` | Stable domain models, settings, errors, and component ports |
| `openclip-storage` | SQLite migrations and metadata repository |
| `openclip-cs2` | CS2 GSI models, adapter, loopback listener, Steam discovery, and config installer |
| `openclip-buffer` | Moment windows plus hard-bounded encoded segment retention/selection |
| `openclip-capture` | Windows target discovery, WGC, H.264/AAC encoding, recovery, and remux |
| `openclip-app` | Thin Tauri host and typed IPC commands |

See [`docs/architecture.md`](docs/architecture.md) for the dependency rules and design rationale.

## Prerequisites

- Windows 10 or 11
- an NVIDIA GPU/driver exposing NVENC H.264 (`nvEncodeAPI64.dll`)
- Rust stable toolchain
- Node.js 22 or newer and npm
- Microsoft C++ Build Tools and WebView2 (required by Tauri on Windows)

## Run locally

Install the frontend dependencies once:

```powershell
cd app
npm install
```

Run the desktop app:

```powershell
npm run tauri dev
```

Run only the browser UI while working on layout:

```powershell
npm run dev
```

The desktop app reads persisted GSI matches through Tauri IPC. `npm run dev` is an explicit browser
preview and uses the fixtures in `app/src/data/developmentFixtures.ts`; IPC failures in the desktop
app are surfaced instead of silently switching to fake data.

## Configure CS2

Open **Settings** in the desktop app and select **Install GSI configuration**. OpenClip discovers
Steam through the Windows registry and configured Steam libraries, then creates or updates only:

```text
game/csgo/cfg/gamestate_integration_openclip.cfg
```

Restart CS2 after installing the configuration. See [`docs/cs2-gsi.md`](docs/cs2-gsi.md) for the
payload contract, diagnostics, and fixture simulation details.

## Automatic highlights and manual replay

1. Start the desktop app; capture continues independently of which React page is open.
2. Launch CS2 in a capturable borderless/fullscreen-windowed or windowed mode.
3. Wait for the sidebar to show **Replay active**.
4. Play normally. Qualifying local-player kills automatically become moments after their post-roll
   expires. Press **F8** at any time for an independent manual replay.

The app remuxes the most recent finalized segments into a dated MP4 under the configured storage
path and writes its metadata to SQLite. The in-flight segment is named `*.mp4.partial` and is never
indexed; an abrupt process termination can lose at most that segment while prior segments remain
independently parseable and are recovered into the next recorder session. See
[`docs/capture.md`](docs/capture.md) for limitations and diagnostics.

See [`docs/moments.md`](docs/moments.md) for merge/finalization rules, scoring, persistence, export,
and capture-unavailable behavior.

## Validate

From the repository root:

```powershell
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd app
npm run typecheck
npm run build
```

SQLite stores metadata only. Captured video remains as ordinary files on disk.

## Roadmap

1. **Repository and core domain** — complete
2. **CS2 GSI listener, installer, state differencing, persistence, and live GUI data** — complete
3. **Windows capture, hardware H.264, bounded manual replay buffer, and F8 save** — complete
4. **automatic merged moments, ranged extraction, real match UI, export, and desktop polish** — complete
5. **direct NVENC, deterministic pacing, crash-safe IDR segments, and microphone mixing** — current
6. montage assembly and editing foundations

OpenClip is licensed under either the MIT License or Apache License 2.0. License files will be added
before the first public release.
