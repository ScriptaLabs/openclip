// Builds the updater manifests for a finished build.
//
//   node scripts/release/make-manifest.mjs --version 1.2.3 --out dist-release \
//        --mirror-base https://cdn.example/openclip --github-base https://github.com/o/r/releases/download/v1.2.3
//
// Writes into --out:
//   latest.json          Tauri updater manifest pointing at the mirror (uploaded next to the installer)
//   latest.github.json   the same manifest pointing at the GitHub Release (attached to the release)
//   update-row.json      the row inserted into the Supabase `update` table
//   <installer>.exe      a copy of the installer, ready to upload
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

function option(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index > 0 ? process.argv[index + 1] : fallback;
}

const version = option("version");
const out = path.resolve(option("out", "dist-release"));
const mirror = option("mirror-base")?.replace(/\/+$/, "");
const mirrorBase = mirror?.startsWith("http") ? mirror : undefined;
const githubBase = option("github-base")?.replace(/\/+$/, "");
const notes = (process.env.RELEASE_NOTES ?? "").trim();
if (!version || !githubBase) {
  console.error("--version and --github-base are required");
  process.exit(1);
}

const bundleDir = path.join(root, "target/release/bundle/nsis");
const installer = fs.readdirSync(bundleDir).find((name) => name.endsWith("-setup.exe"));
if (!installer) throw new Error(`no NSIS installer found in ${bundleDir}`);
const installerPath = path.join(bundleDir, installer);
const signaturePath = `${installerPath}.sig`;
if (!fs.existsSync(signaturePath)) {
  throw new Error("installer signature is missing; was TAURI_SIGNING_PRIVATE_KEY set for the build?");
}

const signature = fs.readFileSync(signaturePath, "utf8").trim();
const sha256 = crypto.createHash("sha256").update(fs.readFileSync(installerPath)).digest("hex");
const pubDate = new Date().toISOString();
const encoded = encodeURIComponent(installer);

function manifest(installerUrl) {
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: { "windows-x86_64": { signature, url: installerUrl } },
  };
}

fs.mkdirSync(out, { recursive: true });
fs.copyFileSync(installerPath, path.join(out, installer));
fs.writeFileSync(path.join(out, "latest.github.json"), `${JSON.stringify(manifest(`${githubBase}/${encoded}`), null, 2)}\n`);

const installerUrl = mirrorBase ? `${mirrorBase}/releases/v${version}/${encoded}` : `${githubBase}/${encoded}`;
fs.writeFileSync(path.join(out, "latest.json"), `${JSON.stringify(manifest(installerUrl), null, 2)}\n`);

// The row is the push signal; the signed manifest remains the source of truth for what installs.
const row = {
  src: {
    version,
    notes,
    pub_date: pubDate,
    channel: "stable",
    installer,
    installer_url: installerUrl,
    github_url: `${githubBase}/${encoded}`,
    sha256,
    commit: process.env.GITHUB_SHA ?? null,
  },
};
fs.writeFileSync(path.join(out, "update-row.json"), `${JSON.stringify(row, null, 2)}\n`);

console.log(`installer ${installer}`);
console.log(`sha256 ${sha256}`);
console.log(`manifest -> ${installerUrl}`);
