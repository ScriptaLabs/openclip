# OpenClip

OpenClip is a lightweight, local-first automatic game clipping application. It is being built for
Counter-Strike 2 on Windows 10/11 with no accounts, cloud dependency, advertisements, social feed,
or telemetry by default.

OpenClip is **match-centric**. Automatic moments belong to rounds and matches; manual replays are
separate clip records which can optionally reference the active GSI match.

## Current milestone

Milestone 3 adds native Windows capture and manual replay saving to the completed CS2 GSI foundation:

- Windows Graphics Capture targeting the largest visible `cs2.exe` window
- GPU-surface input to Windows Media Foundation H.264 with hardware acceleration requested and an
  NVIDIA driver/NVENC capability gate
- process-tree WASAPI loopback audio for CS2, encoded as AAC at 48 kHz stereo
- independently finalized approximately two-second MP4 segments; only parsed, sync-sample-starting
  segments enter the rolling buffer
- hard duration and byte bounds, keyframe-safe selection, and encoded-sample MP4 remuxing on save
- F8 global Save Replay plus the shared `save_replay` Tauri command
- separate persistent manual clip metadata, including active-match association when GSI has one
- launch/close/relaunch monitoring, resolution-change reset, and local capture diagnostics
- persistent quality, resolution, frame-rate, game-audio, and microphone device preferences
- the supplied OpenClip logo/banner and CS2 artwork, plus restrained accessible navigation states

Automatic event-triggered video saving and microphone mixing remain intentionally deferred. The UI
enumerates active Windows microphone endpoints and persists the enable/device preference without
claiming that microphone audio is already present in Milestone 3 clips.

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

## Save a replay

1. Start the desktop app; capture continues independently of which React page is open.
2. Launch CS2 in a capturable borderless/fullscreen-windowed or windowed mode.
3. Wait for the sidebar to show **Replay active**.
4. Play for at least 15 seconds and press **F8**.

The app remuxes the most recent finalized segments into a dated MP4 under the configured storage
path and writes its metadata to SQLite. The in-flight segment is named `*.mp4.partial` and is never
indexed; an abrupt process termination can lose at most that segment while prior segments remain
independently parseable and are recovered into the next recorder session. See
[`docs/capture.md`](docs/capture.md) for limitations and diagnostics.

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
3. **Windows capture, hardware H.264, bounded manual replay buffer, and F8 save** — current
4. automatic merged moments triggered by CS2 events
5. per-match export of interesting moments

OpenClip is licensed under either the MIT License or Apache License 2.0. License files will be added
before the first public release.
