// Stamps one version everywhere it is shown or compared: the installer, the updater, the UI.
// Usage: node scripts/release/set-version.mjs 1.2.3
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const version = process.argv[2]?.replace(/^v/, "");
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error("usage: set-version.mjs <semver>   e.g. 1.2.3 or 1.2.3-beta.1");
  process.exit(1);
}

function rewriteJson(file, change) {
  const target = path.join(root, file);
  const json = JSON.parse(fs.readFileSync(target, "utf8"));
  change(json);
  fs.writeFileSync(target, `${JSON.stringify(json, null, 2)}\n`);
}

rewriteJson("app/src-tauri/tauri.conf.json", (conf) => { conf.version = version; });
rewriteJson("app/package.json", (pkg) => { pkg.version = version; });

// The workspace version feeds every crate; Cargo.lock follows on the next build.
const cargo = path.join(root, "Cargo.toml");
const text = fs.readFileSync(cargo, "utf8");
const updated = text.replace(/(\[workspace\.package\][^\[]*?version\s*=\s*")[^"]+(")/s, `$1${version}$2`);
if (updated === text && !text.includes(`version = "${version}"`)) {
  console.error("could not find [workspace.package] version in Cargo.toml");
  process.exit(1);
}
fs.writeFileSync(cargo, updated);
console.log(`version set to ${version}`);
