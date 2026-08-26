# OpenClip

OpenClip is a lightweight, local-first automatic game clipping application. It is being built for
Counter-Strike 2 on Windows 10/11 with no accounts, cloud dependency, advertisements, social feed,
or telemetry by default.

OpenClip is **match-centric**. A recording belongs to a moment, a moment belongs to a round, and a
round belongs to a match. The product is designed around reviewing a match and exporting its useful
moments—not browsing a flat folder of disconnected video files.

## Current milestone

Milestone 2 adds reliable Counter-Strike 2 Game State Integration to the project foundation:

- Rust workspace with domain, storage, CS2, capture, and replay-buffer boundaries
- strongly typed sessions, matches, rounds, events, moments, recording status, and settings
- SQLite migration and repository for persistent metadata
- window-based moment grouping with configurable pre-roll and post-roll
- Tauri 2 application shell with React, strict TypeScript, Tailwind CSS, and typed IPC commands
- a loopback-only Axum listener on `127.0.0.1:21987`
- partial-tolerant CS2 payload models and stateful match, round, kill, and headshot detection
- Steam library discovery and an idempotent OpenClip-only GSI config installer
- transactional persistence from each derived game-state update
- live Tauri status, setup, match queries, and lifecycle events
- real persisted matches in the desktop UI, with explicit fixtures only in browser preview
- match artwork sourced from the provided `app/assets/map_splash` collection

Video capture, encoding, audio, replay extraction, cloud services, and accounts remain intentionally
out of scope.

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
| `openclip-buffer` | Moment-window logic now; encoded segment ring/extraction later |
| `openclip-capture` | Encoder/audio contracts; native Windows implementation later |
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

SQLite stores metadata only. Captured video will remain as ordinary files on disk and be linked to
matches and moments by IDs.

## Roadmap

1. **Repository and core domain** — complete
2. **CS2 GSI listener, installer, state differencing, persistence, and live GUI data** — current
3. capture orchestration groundwork
4. Windows capture with hardware encoding and a manual replay buffer
5. automatic merged moments triggered by CS2 events
6. per-match export of interesting moments

OpenClip is licensed under either the MIT License or Apache License 2.0. License files will be added
before the first public release.
