import type { GameEvent, MatchRecord, Moment, MomentClassification } from "../domain";
import { weaponLabel } from "../domain";
import { mapLabel } from "../lib/maps";

export interface KillMark {
  n: number;
  weaponRaw: string | null;
  weapon: string | null;
  headshot: boolean;
  /** Approximate position in the clip, from the moment's own timeline. */
  atMs: number;
}

/** A replay saved with the shortcut. It has no moment, so it is keyed by its clip record. */
export interface ManualClipRecord {
  id: string;
  matchId: string | null;
  source: "manual" | "automatic_moment";
  createdAt: string;
  path: string;
  durationMs: number;
  isFavorite: boolean;
}

export interface ClipEntry {
  id: string;
  /** `manual` entries are replays saved with the shortcut; they are keyed by clip, not moment. */
  source: "automatic" | "manual";
  matchId: string;
  moment: Moment;
  match: Pick<MatchRecord, "map" | "mode" | "scoreTeamA" | "scoreTeamB" | "startedAt" | "endedAt">;
  mapName: string;
  roundNumber: number | null;
  kills: KillMark[];
  headshots: number;
  weaponRaw: string | null;
  weapon: string | null;
  badge: string;
  title: string;
  durationMs: number;
  hasClip: boolean;
  favorite: boolean;
  /** Marks four or more kills, which the UI highlights. */
  big: boolean;
  at: number;
}

const badges: Record<MomentClassification, string> = {
  kill: "1K", headshot: "HS", double_kill: "2K", triple_kill: "3K", quad_kill: "4K",
  five_kill: "5K", multi_kill: "MK", clutch: "CL", ace: "ACE", other: "CLIP",
};

function flag(event: GameEvent, name: string): boolean {
  return event.metadata[name] === true;
}

function killMarks(moment: Moment): KillMark[] {
  return moment.events
    .filter((event) => event.eventType === "kill")
    .map((event, index) => {
      const raw = typeof event.metadata.weapon === "string" ? event.metadata.weapon : null;
      const offset = event.matchOffsetMs;
      return {
        n: index + 1,
        weaponRaw: raw,
        weapon: raw ? weaponLabel(raw) : null,
        headshot: flag(event, "headshot"),
        atMs: offset === null ? 0 : Math.max(0, offset - moment.startOffsetMs),
      };
    });
}

function dominantRaw(kills: KillMark[]): string | null {
  const counts = new Map<string, number>();
  for (const kill of kills) {
    if (kill.weaponRaw) counts.set(kill.weaponRaw, (counts.get(kill.weaponRaw) ?? 0) + 1);
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
}

function manualEntry(clip: ManualClipRecord, match: MatchRecord | undefined): ClipEntry {
  const at = new Date(clip.createdAt).getTime();
  const moment: Moment = {
    id: clip.id,
    matchId: clip.matchId ?? "",
    roundId: null,
    startOffsetMs: 0,
    endOffsetMs: clip.durationMs,
    events: [],
    classification: "other",
    importance: 0,
    clipId: clip.id,
    clipPath: clip.path,
    isFavorite: clip.isFavorite,
    title: "Manual clip",
    weapon: "",
    thumbnailTone: "cyan",
  };
  return {
    id: clip.id,
    source: "manual",
    matchId: clip.matchId ?? "",
    moment,
    match: match
      ? {
        map: match.map, mode: match.mode, scoreTeamA: match.scoreTeamA, scoreTeamB: match.scoreTeamB,
        startedAt: match.startedAt, endedAt: match.endedAt,
      }
      : { map: null, mode: "unknown", scoreTeamA: null, scoreTeamB: null, startedAt: clip.createdAt, endedAt: null },
    mapName: match ? mapLabel(match.map) : "No match",
    roundNumber: null,
    kills: [],
    headshots: 0,
    weaponRaw: null,
    weapon: null,
    badge: "REPLAY",
    title: "Manual clip",
    durationMs: clip.durationMs,
    hasClip: true,
    favorite: clip.isFavorite,
    big: false,
    at: Number.isNaN(at) ? 0 : at,
  };
}

/** Moments with their clips, plus the replays saved by hand. */
export function buildClips(matches: MatchRecord[], manual: ManualClipRecord[] = []): ClipEntry[] {
  const entries: ClipEntry[] = [];
  const byId = new Map(matches.map((match) => [match.id, match]));
  for (const clip of manual) {
    if (clip.source === "manual") entries.push(manualEntry(clip, clip.matchId ? byId.get(clip.matchId) : undefined));
  }
  for (const match of matches) {
    const rounds = new Map(match.rounds.map((round) => [round.id, round.number]));
    for (const moment of match.moments) {
      const kills = killMarks(moment);
      const weaponRaw = dominantRaw(kills);
      const occurred = new Date(moment.events[0]?.occurredAt ?? match.startedAt).getTime();
      const at = Number.isNaN(occurred) ? new Date(match.startedAt).getTime() : occurred;
      const knife = moment.events.some((event) => flag(event, "knifeKill"));
      entries.push({
        id: moment.id,
        source: "automatic",
        matchId: match.id,
        moment,
        match: {
          map: match.map, mode: match.mode, scoreTeamA: match.scoreTeamA, scoreTeamB: match.scoreTeamB,
          startedAt: match.startedAt, endedAt: match.endedAt,
        },
        mapName: mapLabel(match.map),
        roundNumber: moment.roundId ? rounds.get(moment.roundId) ?? null : null,
        kills,
        headshots: kills.filter((kill) => kill.headshot).length,
        weaponRaw,
        weapon: weaponRaw ? weaponLabel(weaponRaw) : null,
        badge: knife && kills.length === 1 ? "KNIFE" : badges[moment.classification],
        title: moment.title,
        durationMs: Math.max(0, moment.endOffsetMs - moment.startOffsetMs),
        hasClip: Boolean(moment.clipPath),
        favorite: moment.isFavorite,
        big: kills.length >= 4,
        at,
      });
    }
  }
  return entries.sort((a, b) => b.at - a.at);
}

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export function dayLabel(timestamp: number): string {
  const day = new Date(timestamp);
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  if (day.toDateString() === today.toDateString()) return "Today";
  if (day.toDateString() === yesterday.toDateString()) return "Yesterday";
  return day.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
}

export function timeLabel(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
