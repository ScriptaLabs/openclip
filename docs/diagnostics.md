# Anonymous problem reports

When recording goes wrong, OpenClip can offer to send an anonymous report so the problem can be
fixed. It is opt-in every time: nothing leaves the computer unless the user presses **Send**.

## When it is offered

- The recorder keeps a short in-memory list of problems (`Incident` in `openclip-core`): capture
  could not start or ended with an error, an encoder stopped producing video, a replay segment could
  not be finalized, audio was unavailable, or a detected moment could not be saved.
- A background thread (`app/src-tauri/src/diagnostics.rs`) offers a report only when **no match is in
  progress**, at most once every 20 minutes, and only if *Settings → Startup and updates → Offer
  problem reports* is on. The offer is a banner at the top of the app, never a pop-up over the game.
- The user can review everything first (**Review and send** → *Show everything that would be sent*),
  add an optional note, send, dismiss, or turn the offers off.
- Builds without the update-channel settings (local development) never offer a report.

## What is sent

Exactly the JSON shown in the preview (`ReportPayload`):

| Field | Content |
| --- | --- |
| `app_version`, `os` | OpenClip version, Windows build (for example `Windows 11 25H2 build 26200`) |
| `gpus` | Graphics adapter names, vendor, video memory and driver version |
| `encoder` | The encoder in use when the problem happened |
| `settings` | Capture settings that affect recording (encoder choice, quality, resolution, frame rate, audio on/off, map filters). No paths, names or keys |
| `kinds`, `summary`, `fingerprint` | The kinds of problem, the first error message, and a short hash that groups the same problem |
| `log` | An excerpt of `openclip.log`: warnings, errors, how the encoder started and a few frame-rate samples, from shortly before the first problem |
| `user_note` | The optional note the user typed |

## What is removed first

`app/src-tauri/src/anonymize.rs` runs on the user's computer, before the preview is shown:

- every file path (`C:\Users\<name>\Videos\…` becomes `<path>`), so account and folder names go;
- the Windows user name, computer name and domain, by value, wherever they appear;
- UUIDs, Steam IDs, email addresses, every IP address, tokens and long secrets;
- custom server map titles (`DE_MIRAGE | RETAKE #61 | …` becomes `<custom>`); engine map names stay.

The report carries no install ID, account ID or anything that links one report to the next.

## Where it goes

`public.diagnostic_report` in the project's Supabase database, written with the public anon key baked
in by CI (the same one the update channel uses). The anon key can **only insert**: row-level security
has an insert policy and no select, update or delete policy, the column grants cover only the report
fields (so a sender cannot set `status`), and column checks cap every field's size. Reports are read
in the Supabase dashboard or with the service role.

- `public.diagnostic_report_summary` groups reports by `fingerprint` (count, first and last seen,
  versions, encoders) for triage.
- `status` (`new`, `investigating`, `fixed`, `wontfix`), `fixed_in` and `notes` are for us.

Anyone holding the public key could still send junk reports; the size limits bound the cost.
`scripts/supabase/diagnostic_report_limits.sql` adds two optional protections, applied by hand: a cap of
120 reports a minute, and a daily job that deletes fixed or won't-fix reports after 30 days and
everything else after 180. If abuse ever becomes real, put an Edge Function with per-client rate
limiting in front of the table.

The graphics adapter's name and driver version are included on purpose, since many capture problems
are specific to a card or driver. The preview lists them, and a note box reminds users to keep
personal details out of what they type (the note is scrubbed too, but only for known patterns).

## Changing the schema

The table was created by the `create_diagnostic_report` migration. Keep `ReportPayload`'s field names
in step with its columns; they are sent as-is.
