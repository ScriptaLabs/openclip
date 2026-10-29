const PREFIXES = ["de_", "cs_", "ar_", "dz_", "gd_"];

/**
 * Community and FACEIT-style servers report names like "DE_MIRAGE | RETAKE #61 | TR | XPLAY.GG"
 * or "workshop/3070923343/de_test". The engine map name is what identifies the map.
 * Mirrors `normalize_map_name` in the Rust adapter, so older stored matches look right too.
 */
export function mapKey(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const lower = raw.toLowerCase();
  const token = lower.split(/[^a-z0-9_]+/).find((part) => PREFIXES.some((prefix) => part.startsWith(prefix)));
  if (token) return token;
  const first = lower.split("|")[0]!.trim();
  return first.split("/").pop()!.trim() || null;
}

/** "de_mirage" -> "Mirage" */
export function mapLabel(raw: string | null | undefined): string {
  const key = mapKey(raw);
  if (!key) return "Unknown map";
  const name = key.replace(new RegExp(`^(${PREFIXES.join("|")})`), "").replace(/_/g, " ");
  return name.replace(/\b\w/g, (letter) => letter.toUpperCase());
}

/** The server's own label ("Retake #61 | TR | xplay.gg"), when the map name carried one. */
export function serverLabel(raw: string | null | undefined): string | null {
  if (!raw || !raw.includes("|")) return null;
  const rest = raw.split("|").slice(1).map((part) => part.trim()).filter(Boolean);
  return rest.length > 0 ? rest.join(" · ") : null;
}
