# Clip retention

Saved clips pile up. **Settings → Storage → Clean up old clips** deletes them automatically, by age,
by total size, or both. It is **off by default** and deleting video cannot be undone, so the card
shows what applying the limits would delete before it does anything.

## The rules

- **Age:** clips older than the chosen number of days are deleted.
- **Size:** once the saved clips take more than the chosen number of GiB, the oldest are deleted until
  they fit.
- **Favorites are never deleted.** A clip made from a moment follows that moment's star; a manual
  replay has its own star in the player. So a limit can leave the total above the size limit if
  favorites alone exceed it.
- **New clips are safe.** Clips saved in the last 10 minutes are never deleted, so a tiny limit cannot
  remove a clip before it has been seen.
- Only recorded `.mp4` clip files are ever deleted, together with their `.thumb.jpg`. The replay
  buffer and everything else in the clip folder is left alone.

## What stays

The match, round and moment records stay, so history and statistics are unchanged. A moment whose
clip was deleted simply has no video (it shows as unavailable). Manual replays that are deleted
disappear from the Clips page.

## When it runs

At most once every 30 minutes while OpenClip is running (first run 45 seconds after start), and
immediately when the user presses **Save and delete** in Settings. If a file cannot be removed, for
example because it is open in a player, it keeps its record and is tried again next time.

## Where the logic lives

- `crates/openclip-core/src/retention.rs`: the planner (pure, tested): which clips to delete.
- `app/src-tauri/src/retention.rs`: applies a plan, deletes files, sweeps in the background.
- `MatchRepository::delete_clip` and `set_clip_favorite` in `openclip-storage`.
