export type Game = "counter_strike2";
export type MatchMode = "competitive" | "premier" | "casual" | "deathmatch" | "unknown";
export type GameEventType =
  | "match_start"
  | "match_end"
  | "round_start"
  | "round_end"
  | "kill"
  | "death"
  | "headshot"
  | "bomb_plant"
  | "bomb_defuse"
  | "round_win";
export type MomentClassification =
  | "kill"
  | "headshot"
  | "double_kill"
  | "triple_kill"
  | "quad_kill"
  | "five_kill"
  | "multi_kill"
  | "clutch"
  | "ace"
  | "other";

export interface GameEvent {
  id: string;
  matchId: string;
  roundId: string | null;
  sourceGame: Game;
  eventType: GameEventType;
  occurredAt: string;
  matchOffsetMs: number | null;
  metadata: Record<string, unknown>;
}

export interface Round {
  id: string;
  matchId: string;
  number: number;
  startedAt: string;
  endedAt: string | null;
  events: GameEvent[];
}

export interface Moment {
  id: string;
  matchId: string;
  roundId: string | null;
  startOffsetMs: number;
  endOffsetMs: number;
  events: GameEvent[];
  classification: MomentClassification;
  importance: number;
  clipId: string | null;
  clipPath: string | null;
  isFavorite: boolean;
  title: string;
  weapon: string;
  thumbnailTone: "amber" | "cyan" | "violet" | "green";
}

export interface MatchRecord {
  id: string;
  sessionId: string;
  map: string | null;
  mode: MatchMode;
  startedAt: string;
  endedAt: string | null;
  scoreTeamA: number | null;
  scoreTeamB: number | null;
  rounds: Round[];
  moments: Moment[];
  thumbnailTone: "mirage" | "inferno" | "ancient";
}

export interface MatchStats {
  kills: number;
  headshots: number;
  highlights: number;
}

export function getMatchStats(match: MatchRecord): MatchStats {
  const events = match.rounds.flatMap((round) => round.events);
  return {
    kills: events.filter((event) => event.eventType === "kill").length,
    headshots: events.filter((event) => event.eventType === "headshot").length,
    highlights: match.moments.length,
  };
}

export function formatMode(mode: MatchMode): string {
  return mode === "unknown" ? "Unknown mode" : mode[0]!.toUpperCase() + mode.slice(1);
}

function eventFlag(event: GameEvent, name: string): boolean {
  return event.metadata[name] === true;
}

export function momentTitle(moment: Pick<Moment, "classification" | "events">): string {
  const kill = moment.events.find((event) => event.eventType === "kill");
  const base = ({
    kill: "Kill", headshot: "Headshot", double_kill: "Double Kill", triple_kill: "Triple Kill",
    quad_kill: "Quad Kill", five_kill: "Five Kill", multi_kill: "Multi-kill", clutch: "Clutch",
    ace: "Ace", other: "Highlight",
  } as const)[moment.classification];
  if (!kill || moment.events.filter((event) => event.eventType === "kill").length > 1) return base;
  const traits = [
    eventFlag(kill, "aerial") ? "Aerial" : "",
    eventFlag(kill, "noScope") ? "No-scope" : "",
    eventFlag(kill, "oneShotOneKill") ? "One-shot-one-kill" : "",
  ].filter(Boolean);
  const finish = eventFlag(kill, "knifeKill")
    ? "Knife Kill"
    : eventFlag(kill, "zeusKill")
      ? "Zeus Kill"
      : eventFlag(kill, "headshot") ? "Headshot" : base;
  return [...traits, finish].join(" · ");
}

const weaponLabels: Record<string, string> = {
  ak47: "AK-47", m4a1: "M4A4", m4a1_silencer: "M4A1-S", awp: "AWP", aug: "AUG", sg556: "SG 553",
  famas: "FAMAS", galilar: "Galil AR", ssg08: "SSG 08", scar20: "SCAR-20", g3sg1: "G3SG1",
  deagle: "Desert Eagle", usp_silencer: "USP-S", hkp2000: "P2000", glock: "Glock-18", p250: "P250",
  fiveseven: "Five-SeveN", tec9: "Tec-9", cz75a: "CZ75-Auto", elite: "Dual Berettas", revolver: "R8 Revolver",
  mp9: "MP9", mac10: "MAC-10", mp7: "MP7", mp5sd: "MP5-SD", ump45: "UMP-45", p90: "P90", bizon: "PP-Bizon",
  nova: "Nova", xm1014: "XM1014", mag7: "MAG-7", sawedoff: "Sawed-Off", m249: "M249", negev: "Negev",
  taser: "Zeus x27", hegrenade: "HE Grenade", inferno: "Incendiary", molotov: "Molotov", knife: "Knife",
};

export function weaponLabel(raw: string): string {
  const key = raw.replace(/^weapon_/, "");
  if (weaponLabels[key]) return weaponLabels[key];
  if (key.includes("knife") || key.includes("bayonet")) return "Knife";
  return key.replace(/_/g, " ").toUpperCase();
}

export function killCount(moment: Pick<Moment, "events">): number {
  return moment.events.filter((event) => event.eventType === "kill").length;
}

/** The weapon behind most kills in a moment, as a display label. */
export function dominantWeapon(moment: Pick<Moment, "events">): string | null {
  const counts = new Map<string, number>();
  for (const event of moment.events) {
    if (event.eventType !== "kill" || typeof event.metadata.weapon !== "string") continue;
    counts.set(event.metadata.weapon, (counts.get(event.metadata.weapon) ?? 0) + 1);
  }
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
  return top ? weaponLabel(top[0]) : null;
}
