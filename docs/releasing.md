# Releasing and updating

OpenClip ships as a per-user Windows installer and updates itself silently.

```
git tag vX.Y.Z ──► GitHub Actions: Release
                    1. checks (fmt, clippy, tests, frontend build)
                    2. stamp the version, build the NSIS installer, sign it with the updater key
                    3. upload the installer + latest.json to Backblaze (public bucket, fast)
                    4. publish the GitHub Release (installer + latest.json)
                    5. INSERT a row into Supabase `update`   ← "a release exists" signal

OpenClip (installed per user, starts with Windows, lives in the tray)
   Supabase realtime INSERT, app start, and every 6 hours
        └► read latest.json (Backblaze first, GitHub as fallback)
        └► verify the signature against the public key built into the app
        └► download in the background (thin progress line in the sidebar)
        └► install silently once no match is running, then relaunch
```

## Why it is built this way

- **Installer is per-user** (`installMode: currentUser`). No administrator prompt, so updates
  can run with no UI at all. Launch-with-Windows is enabled on a fresh install and can be turned
  off in Settings; updates never switch it back on.
- **Signature, not transport, is the trust anchor.** The updater key pair is separate from
  everything else. The app only installs an archive signed by the matching private key, so a
  compromised bucket, a spoofed realtime message or a stray database row cannot install code.
- **Supabase is only a doorbell.** A row (`src` = version, notes, URLs, sha256) wakes running
  apps instantly. The app does not trust it: it re-reads the signed manifest.
- **Two download sources.** Backblaze is faster and cheaper for bandwidth; the GitHub Release is
  the always-there fallback. The updater tries them in that order.
- **Never mid-match.** An update that is ready waits while CS2 is in a match or a clip is
  being saved (`MatchInProgress` / `SavingReplay`), and installs right after. The sidebar says
  so. A match that stops reporting (CS2 closed or crashed) counts as finished after 60 seconds.
- **Newest wins, announced last.** CI inserts the Supabase row only after the files are
  downloadable, so nobody is told about a release that is not there yet.

## One-time setup

### 1. Supabase

Run `supabase/migrations/20261029000000_update_channel.sql` in the SQL editor. It enables row
level security, lets anyone **read** releases (the app holds only the public anon key), allows no
public writes, and turns on realtime for the table.

`SUPABASE_SUBSCRIBE` can be the realtime websocket URL
(`wss://<project>.supabase.co/realtime/v1/websocket?apikey=<anon key>`), or any URL on the
project, such as `https://<project>.supabase.co/rest/v1/update`. Only its host is used. In that
case also set the `SUPABASE_ANON_KEY` secret to the project's public (anon) key, which the app
needs to listen for releases. Row level security keeps that key read-only.

### 2. Backblaze B2

Create a **public** bucket and an application key limited to it. Note the S3 endpoint
(`https://s3.<region>.backblazeb2.com`) and the public base URL
(for example `https://f005.backblazeb2.com/file/<bucket>`, or your CDN in front of it).

### 3. GitHub: Settings → Secrets and variables → Actions

| Kind | Name | What |
| --- | --- | --- |
| Secret | `TAURI_SIGNING_PRIVATE_KEY` | contents of the updater private key (see below) |
| Secret | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the key's password (empty if none) |
| Secret | `SUPABASE_SUBSCRIBE` | realtime URL baked into the app |
| Secret | `SUPABASE_ANON_KEY` | only if `SUPABASE_SUBSCRIBE` is the project URL |
| Secret | `SUPABASE_WRITE_KEY` | key that can only INSERT into `public.update` |
| Secret | `BACKBLAZE_KEYID` | B2 application key ID |
| Secret | `BACKBLAZE_APPKEY` | B2 application key |
| Variable | `B2_BUCKET` | bucket name |
| Variable | `B2_ENDPOINT` | `https://s3.<region>.backblazeb2.com` |
| Variable | `B2_PUBLIC_BASE_URL` | public base URL of the bucket |
| Variable | `SUPABASE_URL` | optional; derived from `SUPABASE_SUBSCRIBE` when unset |

### 4. The updater key

The public key is committed in `app/src-tauri/tauri.conf.json`. The private key is **not** in the
repository. To make a new pair:

```bash
cd app && npx tauri signer generate -w ~/.openclip-keys/updater.key
```

Then put the new public key in `tauri.conf.json` and the private key in the
`TAURI_SIGNING_PRIVATE_KEY` secret. **If the private key is lost or replaced, installed apps
cannot verify new releases and must be reinstalled by hand.** Keep a backup.

## Shipping a release

```bash
git tag v1.2.0
git push origin v1.2.0
```

- `v1.2.0` goes to Backblaze, GitHub and the update channel; running apps update themselves.
- `v1.2.0-beta.1` (any suffix) is a GitHub **pre-release** only. It never reaches the channel.
- Release notes are the commit subjects since the previous tag.
- The version is taken from the tag. You do not edit version numbers by hand.

## Testing an update locally

1. Install an older build (a tag, or a local `npx tauri build` with an older `version`).
2. Publish a newer tag.
3. The older app downloads the update in the background and installs it when no match is
   running. Settings → Startup and updates shows the state, and has **Check for updates**.

## Files

| Path | Role |
| --- | --- |
| `.github/workflows/ci.yml` | fmt, clippy, tests, frontend checks (also run before every release) |
| `.github/workflows/release.yml` | tag → build → sign → Backblaze → GitHub Release → Supabase |
| `scripts/release/set-version.mjs` | stamps the tag's version into the config files |
| `scripts/release/configure.mjs` | puts the Backblaze mirror first in the updater endpoints |
| `scripts/release/make-manifest.mjs` | builds `latest.json` (mirror and GitHub flavours) and the Supabase row |
| `scripts/release/publish-supabase.mjs` | inserts the row (last step) |
| `app/src-tauri/installer/` | installer artwork and NSIS hooks |
| `app/src-tauri/src/updater.rs` | download, verify, install-when-safe |
| `app/src-tauri/src/realtime.rs` | Supabase realtime listener |
| `supabase/migrations/` | table policies and realtime |

## LFS note

`dependencies/ffmpeg/ffmpeg.exe` is stored in Git LFS (about 100 MB). CI checks do not fetch it;
only releases do, and its LFS objects are cached so releases do not exhaust the free LFS
bandwidth quota.
