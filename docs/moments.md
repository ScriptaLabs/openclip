# Automatic moments

OpenClip turns persisted, generic game events into logical Moments before it asks the recorder for
video. The CS2 adapter discovers kills and dependable headshot deltas; it does not select replay
segments or write clips.

## Lifecycle and boundaries

A qualifying kill at capture time `T` opens a window from `T - pre_roll` through
`T + post_roll` (saturating at zero). A later qualifying kill in the same match and round extends
the active end to its own post-roll. With multi-kills disabled, a later kill closes the current
window instead of merging it. A timer uses channel receive deadlines rather than a polling loop and
closes the Moment only after the last event's post-roll expires.

Round end/start and match end/start close pending state at the boundary. Events from different
rounds or matches never merge. Kill capture, known-headshot interest, multi-kill merging, pre-roll,
and post-roll all use the persisted settings. Headshots are shown/scored only when GSI supplied a
dependable transition.

Final classification is based on known kills in the Moment: Kill, Double Kill, Triple Kill, Quad
Kill, or Five Kill. OpenClip deliberately does not call a five-kill window an Ace because current GSI
data does not prove that those kills eliminated every opposing player in the round.

## Importance

Scoring is deterministic and independent of GSI parsing, capture, and UI rendering:

- each kill: +10
- each known headshot: +5
- double-kill bonus: +10
- triple-kill bonus: +25
- quad-kill bonus: +40
- five-kill bonus: +60

The score supports predictable interesting-moment export and future ranking without discarding the
underlying event references.

## Timing and the engine

`MomentEngine` (in `openclip-buffer`) owns window grouping and the requested capture range. It takes
the recorder timeline clock as an argument and re-stamps every event from the moment its GSI update
was observed, so grouping and the saved range share one clock rather than the whole-second GSI
timestamp. The desktop host only feeds it updates and persists its jobs. The adapter attributes a
kill that arrives in the same payload as round-over or game-over to the round and match that are
still open, so the round-winning kill stays inside its multi-kill and the match-winning kill is not
lost.

`crates/openclip-cs2/tests/replay_pipeline.rs` replays timed GSI payloads through the real adapter
and engine on a fake clock. New edge cases (FACEIT, reconnects) should be added there first.

## Capture status

Each finalized automatic moment reports `captured`, `partial` or `failed`. `partial` means a clip
exists but the retained buffer began after the requested start or skipped footage between segments;
`missingMs` carries the missing lead-in. `failed` means no clip was saved and the moment is kept
without one.

## Capture and persistence

When a Moment closes, the save worker requests its exact logical capture range. The rolling buffer
selects every complete, independently finalized segment intersecting that range, including safe
keyframe boundary overhang. If the newest segment is still active, the save waits on finalization for
up to five seconds. Selected paths are pinned against pruning during encoded-sample remux; H.264/AAC
is copied into a new MP4 without decoding or re-encoding.

A successful automatic save atomically persists the Moment and its `AutomaticMoment` Clip origin.
Event relationships, match/round IDs, classification, score, favorite state, and clip relationship
remain queryable. If capture, synchronization, or disk writing fails, the logical Moment still
persists without a clip. The UI represents that honestly as **Clip unavailable**.

F8 uses the recent 15-second selector and creates a `Manual` Clip. It does not participate in moment
classification or importance scoring.

## UI and export

Game Detail reads persisted Moments and Clips. Highlight rows open only database-resolved clip paths
through the OS media player, and round markers derive from actual Moment relationships. Automatic
completion emits a restrained in-app notice when enabled.

**Export interesting moments** copies captured automatic/favorited qualifying moments from one Match
to a native-picker destination. Names contain sanitized map, round, classification, and timestamp
metadata. Existing files are never overwritten; collisions receive ` (2)`, ` (3)`, and so on.

Changing the clip storage folder is validated with a real create/write/delete probe and applies to
future manual and automatic clips. It does not move the active temporary replay buffer or existing
clips.

## Known limitations

- Optional microphone capture uses the selected endpoint and mixes into the same AAC clip track as
  CS2 process audio.
- Five Kill is intentionally conservative; true Ace detection needs stronger round/opponent proof.
- Automatic moments require GSI and capture clocks to be active in the same desktop process.
- Export copies separate clips; montage rendering remains deferred.
