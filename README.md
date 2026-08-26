# OpenClip

OpenClip is a lightweight, local-first automatic game clipping application. It is being built for
Counter-Strike 2 on Windows 10/11 with no accounts, cloud dependency, advertisements, social feed,
or telemetry by default.

OpenClip is **match-centric**. A recording belongs to a moment, a moment belongs to a round, and a
round belongs to a match. The product is designed around reviewing a match and exporting its useful
moments—not browsing a flat folder of disconnected video files.

## Current milestone

Milestone 1 establishes the project foundation:

- Rust workspace with domain, storage, CS2, capture, and replay-buffer boundaries
- strongly typed sessions, matches, rounds, events, moments, recording status, and settings
- SQLite migration and repository for persistent metadata
- window-based moment grouping with configurable pre-roll and post-roll
- Tauri 2 application shell with React, strict TypeScript, Tailwind CSS, and typed IPC commands
- polished Games, Game Detail, Montages, and Settings views
- typed development match fixtures behind a replaceable frontend repository interface

Video capture and the CS2 GSI listener are intentionally not implemented yet. The next milestone is
CS2 GSI ingestion and reliable event detection.

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
| `openclip-cs2` | CS2 GSI boundary; listener and adapter arrive in Milestone 2 |
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

The Milestone 1 UI uses fixture matches defined in
`app/src/data/developmentFixtures.ts`. They are exposed through `MatchRepository`, making the data
source explicit and easy to replace with persisted GSI matches in a later milestone.

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

1. **Repository and core domain** — current milestone
2. CS2 GSI listener, configuration installer, state differencing, and event persistence
3. GUI backed by real match data
4. Windows capture with hardware encoding and a manual replay buffer
5. automatic merged moments triggered by CS2 events
6. per-match export of interesting moments

OpenClip is licensed under either the MIT License or Apache License 2.0. License files will be added
before the first public release.

