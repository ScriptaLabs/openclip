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
