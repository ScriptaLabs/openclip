// Announces a finished release by inserting a row into the Supabase `update` table.
// Running copies of OpenClip are subscribed to those inserts and check for the update at once.
//
//   SUPABASE_WRITE_KEY   a key allowed to INSERT into public.update (never shipped to users)
//   SUPABASE_URL         https://<project>.supabase.co (derived from SUPABASE_SUBSCRIBE when unset)
//
// Nothing here prints a key. This must run last: only announce what is already downloadable.
import fs from "node:fs";

const rowFile = process.argv[2] ?? "dist-release/update-row.json";
const key = process.env.SUPABASE_WRITE_KEY;

function projectUrl() {
  if (process.env.SUPABASE_URL) return process.env.SUPABASE_URL.replace(/\/+$/, "");
  const subscribe = process.env.SUPABASE_SUBSCRIBE;
  if (!subscribe) return null;
  const url = new URL(subscribe);
  url.protocol = "https:";
  return `${url.protocol}//${url.host}`;
}

const base = projectUrl();
if (!key || !base) {
  console.error("SUPABASE_WRITE_KEY and SUPABASE_URL (or SUPABASE_SUBSCRIBE) are required");
  process.exit(1);
}

const response = await fetch(`${base}/rest/v1/update`, {
  method: "POST",
  headers: {
    apikey: key,
    Authorization: `Bearer ${key}`,
    "Content-Type": "application/json",
    Prefer: "return=minimal",
  },
  body: fs.readFileSync(rowFile, "utf8"),
});

if (!response.ok) {
  console.error(`Supabase rejected the update row: HTTP ${response.status} ${await response.text()}`);
  process.exit(1);
}
console.log("release announced on the update channel");
