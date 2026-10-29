import type { MatchRecord } from "../domain";
import { weaponLabel } from "../domain";
import type { ClipEntry } from "./clips";

export interface RoundBar {
  number: number;
  kills: number;
  /** A clip was saved for this round. */
  saved: boolean;
  clip: ClipEntry | null;
}

export interface WeaponStat {
  raw: string;
  label: string;
  kills: number;
}

export interface MatchSummary {
  match: MatchRecord;
  clips: ClipEntry[];
  kills: number;
  headshots: number;
  headshotPercent: number;
  multiKills: number;
  clipsSaved: number;
  rounds: RoundBar[];
  weapons: WeaponStat[];
  best: ClipEntry | null;
}

export function buildMatchSummary(match: MatchRecord, allClips: ClipEntry[]): MatchSummary {
  const clips = allClips.filter((clip) => clip.matchId === match.id);
  const killEvents = match.rounds.flatMap((round) => round.events.filter((event) => event.eventType === "kill"));
  const headshots = killEvents.filter((event) => event.metadata.headshot === true).length;

  const weaponCounts = new Map<string, number>();
  for (const event of killEvents) {
    const raw = event.metadata.weapon;
    if (typeof raw === "string") weaponCounts.set(raw, (weaponCounts.get(raw) ?? 0) + 1);
  }
  const weapons = [...weaponCounts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 5)
    .map(([raw, kills]) => ({ raw, label: weaponLabel(raw), kills }));

  const rounds = [...match.rounds]
    .sort((a, b) => a.number - b.number)
    .map((round) => {
      const clip = clips.find((entry) => entry.moment.roundId === round.id && entry.hasClip) ?? null;
      return {
        number: round.number,
        kills: round.events.filter((event) => event.eventType === "kill").length,
        saved: clip !== null,
        clip,
      };
    });

  const playable = clips.filter((clip) => clip.hasClip);
  return {
    match,
    clips,
    kills: killEvents.length,
    headshots,
    headshotPercent: killEvents.length ? Math.round((headshots / killEvents.length) * 100) : 0,
    multiKills: clips.filter((clip) => clip.kills.length >= 2).length,
    clipsSaved: playable.length,
    rounds,
    weapons,
    best: [...playable].sort((a, b) => b.moment.importance - a.moment.importance)[0] ?? null,
  };
}

export interface Overview {
  /** The most recent match. */
  summary: MatchSummary | null;
  recent: ClipEntry[];
  /** Moments across all matches, split by whether a clip exists. */
  health: { captured: number; missed: number };
}

export function buildOverview(matches: MatchRecord[], clips: ClipEntry[]): Overview {
  const latest = [...matches].sort(
    (a, b) => new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
  )[0];
  return {
    summary: latest ? buildMatchSummary(latest, clips) : null,
    recent: clips.filter((clip) => clip.hasClip).slice(0, 4),
    health: {
      captured: clips.filter((clip) => clip.hasClip).length,
      missed: clips.filter((clip) => !clip.hasClip).length,
    },
  };
}
