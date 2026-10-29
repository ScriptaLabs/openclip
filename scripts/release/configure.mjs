// Puts the fast mirror in front of GitHub in the updater's endpoint list.
// Usage: OPENCLIP_DOWNLOAD_BASE=https://cdn.example/openclip node scripts/release/configure.mjs
//
// The updater tries endpoints in order, so installs normally download from the mirror and fall
// back to the GitHub Release if it is unreachable. Nothing secret is written: the endpoints and the
// signature public key are public by design.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const file = path.join(root, "app/src-tauri/tauri.conf.json");
const configured = process.env.OPENCLIP_DOWNLOAD_BASE?.replace(/\/+$/, "");
// A blank repository variable arrives as just "/openclip"; only a real URL counts.
const base = configured?.startsWith("http") ? configured : undefined;

if (!base) {
  console.log("OPENCLIP_DOWNLOAD_BASE is not set; keeping the GitHub endpoint only");
  process.exit(0);
}

const conf = JSON.parse(fs.readFileSync(file, "utf8"));
const mirror = `${base}/latest.json`;
const endpoints = conf.plugins.updater.endpoints.filter((endpoint) => endpoint !== mirror);
conf.plugins.updater.endpoints = [mirror, ...endpoints];
fs.writeFileSync(file, `${JSON.stringify(conf, null, 2)}\n`);
console.log(`updater endpoints: ${conf.plugins.updater.endpoints.join(", ")}`);
