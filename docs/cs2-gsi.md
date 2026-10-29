# Counter-Strike 2 GSI

OpenClip receives CS2 Game State Integration snapshots at `http://127.0.0.1:21987/`. The listener is
hard-coded to IPv4 loopback and rejects a pre-bound non-loopback socket. Requests are limited to one
MiB. Valid payloads return HTTP 204, malformed JSON returns 400, and persistence failures return 500.

## Installation

Use **Settings → Install GSI configuration** in the desktop application. Discovery checks the Steam
Windows registry, common Steam locations, `steamapps/libraryfolders.vdf`, and app manifest 730. The
installer owns only `gamestate_integration_openclip.cfg`; it does not edit or remove another tool's
configuration. Re-running it is safe and reports that current content is already installed.

The requested GSI sections are intentionally focused: provider, map, round, player identity, player
state, weapons, and match statistics. Restart CS2 after the file is installed.

## State interpretation

GSI is a stream of partial snapshots rather than an event log. The adapter therefore keeps its last
accepted state and derives transitions. It creates match and round boundaries from live/over phases,
uses score and round resets to identify a fresh match, and treats a map change as the end of one
match and the start of another. Kill totals are baselined on first observation, equal snapshots are
ignored, missing fields preserve the baseline, and decreasing counters reset it without inventing a
kill. Headshots use the per-round headshot delta when CS2 supplies it.

Only local-player statistics are accepted: matching provider/player Steam IDs are preferred, with
`activity: playing` as the fallback for partial payloads.

After each update is transactionally persisted, the Tauri observer forwards its generic
`GameEvent`s to the automatic moment service. Kill events retain their match/round identity and
known headshot metadata. The moment layer—not the CS2 adapter—applies capture settings, merge
windows, classification, and importance scoring. This keeps the replay service game-agnostic and
prevents a pending moment from crossing a round or match boundary. See
[`moments.md`](moments.md) for the complete lifecycle.

## Tests and simulation

Sanitized payloads live in `crates/openclip-cs2/tests/fixtures`. The integration suite starts an
actual port-zero loopback listener, posts valid and malformed payloads, and verifies the resulting
SQLite graph:

```powershell
cargo test -p openclip-cs2
```

For a running desktop app, a fixture can also be posted manually:

```powershell
Invoke-WebRequest -Method Post -ContentType "application/json" `
  -InFile "crates/openclip-cs2/tests/fixtures/round_live.json" `
  -Uri "http://127.0.0.1:21987/"
```

Structured native logs report listener startup, CS2 detection, lifecycle transitions, malformed
payloads, and persistence failures. The Settings card distinguishes a missing Steam/CS2 install from
an install that simply needs OpenClip's config.

## Game modes

`map.mode` is mapped to the generic `MatchMode`: `competitive`, `premier`, `casual`, `deathmatch`
and `rush`. Any other value is stored as `unknown` and logged once as "unrecognized game mode" so
it can be added to `parse_mode`; the match is still recorded.

Rush (3v3 against bots, about 40-second rounds, 15 rounds, no halftime, friendly fire on) is the
game's own `rush` mode in `gamemodes.txt`. Its map name has no engine prefix (`rush_001`); map names
are normalized to the engine name, and a trailing revision such as `_001` is hidden in the UI.
