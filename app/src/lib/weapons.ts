const weaponFiles = import.meta.glob("../../assets/cs2-icons/weapons/*.svg", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

const killfeedFiles = import.meta.glob("../../assets/cs2-icons/killfeed/*.svg", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

function indexByName(files: Record<string, string>): Map<string, string> {
  return new Map(
    Object.entries(files).map(([path, url]) => [path.split("/").pop()!.replace(/\.svg$/, ""), url]),
  );
}

const weapons = indexByName(weaponFiles);
const killfeed = indexByName(killfeedFiles);

/** "weapon_ak47" -> "ak47" */
export function weaponKey(raw: string): string {
  return raw.replace(/^weapon_/, "");
}

/** The CS2 killfeed icon for a weapon, if the game ships one. */
export function weaponIcon(raw: string | null | undefined): string | undefined {
  if (!raw) return undefined;
  const key = weaponKey(raw);
  const direct = weapons.get(key);
  if (direct) return direct;
  if (key.includes("knife") || key.includes("bayonet")) return weapons.get("knife");
  return undefined;
}

export const headshotIcon = killfeed.get("icon_headshot");
