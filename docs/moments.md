# Automatic moments

OpenClip turns persisted, generic game events into logical Moments before it asks the recorder for
video. The CS2 adapter discovers kills and dependable headshot deltas; it does not select replay
segments or write clips.

## Lifecycle and boundaries

A qualifying kill at capture time `T` opens a window from `T - pre_roll` through
`T + post_roll` (saturating at zero). A later qualifying kill in the same match and round joins the
same Moment when it lands within the **multi-kill window** of the previous kill (default 8 s,
adjustable from 2 s to 30 s). The window is independent of the post-roll: a short clip tail never
makes grouping stricter. Each kill moves the clip's end to its own `post_roll`, while the Moment
stays open for the full window in case another kill follows, then closes and is saved. A single
multi-kill spans at most 60 s, so an unbroken chain (deathmatch, retakes) becomes several clips and
no kill is dropped. With multi-kills disabled, a later kill closes the current window instead of
merging it. A timer uses channel receive deadlines rather than a polling loop.

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

## Capture overlay

After a moment is finalized and saved, `overlay.rs` shows a separate transparent, click-through,
non-focusable window in the top-left (or top-right) of the primary monitor. It is hidden whenever
nothing is on screen and never appears before the moment's post-roll has been recorded. It shows
the logo, the moment, and `captured` / `partial` / `failed`, optionally followed by a muted 2-3
second preview of the saved clip, then fades out. Sounds (`assets/sfx`) play with it; volume and an
on/off switch are in Settings. The window is granted read access to exactly the one clip it
previews. Overlays cannot draw over exclusive fullscreen; CS2 must run borderless or windowed.

`npm run dev` with `?overlay=captured|partial|failed` (optionally `&clip=<url>&side=right`)
previews the overlay in a browser.

## Workshop maps and community servers

Practice on a workshop map (an aim trainer, a prefire map) can produce hundreds of kills an hour.
Two settings, both on by default, keep those out of the automatic clips:

- **Skip workshop maps** for workshop and other custom maps.
- **Skip community servers** for community servers.

Each match is labelled when it starts (`MatchOrigin`: `official`, `workshop` or `community`), and the
moment engine drops the events of any match whose origin is currently skipped. The setting applies
immediately, including to a match already running. F8 is never filtered, and the match is still
recorded so it can be opened from the Matches page if it did produce a clip. Matches of a skipped
origin that produced nothing are hidden from the list.

The game state does not say which server it is, so the label is read from the map name
(`classify_origin` in `openclip-cs2`):

| Map name the game reports | Label |
| --- | --- |
| `workshop/<id>/<map>` | workshop |
| a plain name without an official prefix, such as `aim_botz` | workshop |
| a name with extras, such as `DE_MIRAGE \| RETAKE #61 \| TR` | community |
| a plain `de_`, `cs_`, `ar_`, `dz_`, `gd_` or `rush_` name | official |

Matchmaking and FACEIT report plain official names, so they are never filtered. The catch: a
community server that keeps a plain official map name looks exactly like matchmaking and cannot be
told apart, so it is clipped. Origins are stored in the `origin` column of `matches` (added by the
storage crate when missing; older matches count as official).
