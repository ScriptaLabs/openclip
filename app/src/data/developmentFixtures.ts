import type { GameEvent, MatchRecord, Moment, Round } from "../domain";

const baseDate = "2026-08-26";

function gameEvent(
  matchId: string,
  roundId: string,
  index: number,
  eventType: GameEvent["eventType"],
  matchOffsetMs: number,
  weapon = "weapon_ak47",
): GameEvent {
  return {
    id: `${matchId}-event-${index}`,
    matchId,
    roundId,
    sourceGame: "counter_strike2",
    eventType,
    occurredAt: `${baseDate}T20:${String(10 + index).padStart(2, "0")}:00Z`,
    matchOffsetMs,
    metadata: { weapon },
  };
}

function makeRound(matchId: string, number: number, killCount: number, headshots: number): Round {
  const roundId = `${matchId}-round-${number}`;
  const events: GameEvent[] = [];
  for (let index = 0; index < killCount; index += 1) {
    const offset = number * 90_000 + index * 3_000;
    events.push(gameEvent(matchId, roundId, number * 10 + index, "kill", offset));
    if (index < headshots) {
      events.push(gameEvent(matchId, roundId, number * 20 + index, "headshot", offset, "weapon_ak47"));
    }
  }
  return {
    id: roundId,
    matchId,
    number,
    startedAt: `${baseDate}T20:${String(number).padStart(2, "0")}:00Z`,
    endedAt: `${baseDate}T20:${String(number).padStart(2, "0")}:58Z`,
    events,
  };
}

function makeMoment(
  matchId: string,
  round: Round,
  index: number,
  title: string,
  weapon: string,
  tone: Moment["thumbnailTone"],
): Moment {
  const events = round.events.filter((event) => event.eventType === "kill" || event.eventType === "headshot");
  const firstOffset = events[0]?.matchOffsetMs ?? round.number * 90_000;
  return {
    id: `${matchId}-moment-${index}`,
    matchId,
    roundId: round.id,
    startOffsetMs: Math.max(0, firstOffset - 10_000),
    endOffsetMs: firstOffset + 8_000,
    events,
    classification: events.filter((event) => event.eventType === "kill").length >= 3 ? "triple_kill" : events.filter((event) => event.eventType === "kill").length === 2 ? "double_kill" : "kill",
    importance: 20 + events.length * 10,
    clipPath: "fixture-clip.mp4",
    clipId: null,
    isFavorite: index === 1,
    title,
    weapon,
    thumbnailTone: tone,
  };
}

const mirageRounds = [
  makeRound("mirage-01", 1, 0, 0),
  makeRound("mirage-01", 2, 1, 1),
  makeRound("mirage-01", 3, 2, 1),
  makeRound("mirage-01", 4, 0, 0),
  makeRound("mirage-01", 5, 1, 0),
  makeRound("mirage-01", 6, 1, 1),
  makeRound("mirage-01", 7, 3, 2),
  makeRound("mirage-01", 8, 0, 0),
  makeRound("mirage-01", 9, 1, 1),
  makeRound("mirage-01", 10, 2, 1),
  makeRound("mirage-01", 11, 0, 0),
  makeRound("mirage-01", 12, 1, 1),
  makeRound("mirage-01", 13, 1, 0),
  makeRound("mirage-01", 14, 2, 1),
  makeRound("mirage-01", 15, 0, 0),
  makeRound("mirage-01", 16, 1, 0),
  makeRound("mirage-01", 17, 0, 0),
  makeRound("mirage-01", 18, 1, 0),
  makeRound("mirage-01", 19, 0, 0),
  makeRound("mirage-01", 20, 1, 0),
  makeRound("mirage-01", 21, 0, 0),
];

const mirageMoments = [
  makeMoment("mirage-01", mirageRounds[6]!, 1, "Triple kill", "AK-47", "amber"),
  makeMoment("mirage-01", mirageRounds[11]!, 2, "AWP headshot", "AWP", "cyan"),
  makeMoment("mirage-01", mirageRounds[13]!, 3, "Site hold", "M4A1-S", "violet"),
];

function compactMatch(
  id: string,
  map: string,
  mode: MatchRecord["mode"],
  score: [number, number],
  tone: MatchRecord["thumbnailTone"],
  startedAt: string,
  killsPerRound: number[],
): MatchRecord {
  const rounds = killsPerRound.map((kills, index) => makeRound(id, index + 1, kills, kills > 0 ? 1 : 0));
  const momentRounds = rounds.filter((round) => round.events.some((event) => event.eventType === "kill"));
  const moments = momentRounds.slice(0, 2).map((round, index) =>
    makeMoment(id, round, index + 1, index === 0 ? "Opening pick" : "Clean hold", index === 0 ? "AWP" : "M4A1-S", index === 0 ? "green" : "violet"),
  );
  return {
    id,
    sessionId: `${id}-session`,
    map,
    mode,
    startedAt,
    endedAt: startedAt,
    scoreTeamA: score[0],
    scoreTeamB: score[1],
    rounds,
    moments,
    thumbnailTone: tone,
  };
}

export const developmentMatches: MatchRecord[] = [
  {
    id: "mirage-01",
    sessionId: "session-01",
    map: "de_mirage",
    mode: "competitive",
    startedAt: `${baseDate}T20:00:00Z`,
    endedAt: `${baseDate}T20:34:00Z`,
    scoreTeamA: 13,
    scoreTeamB: 8,
    rounds: mirageRounds,
    moments: mirageMoments,
    thumbnailTone: "mirage",
  },
  compactMatch("inferno-02", "de_inferno", "premier", [11, 13], "inferno", `${baseDate}T16:20:00Z`, [1, 0, 2, 0, 1, 0, 1, 0]),
  compactMatch("rush-04", "rush_001", "rush", [15, 11], "mirage", `${baseDate}T21:10:00Z`, [2, 1, 0, 3, 1, 1, 2, 0, 1, 2, 0, 1, 1, 0, 2]),
  compactMatch("ancient-03", "DE_ANCIENT | RETAKE #61 | TR | XPLAY.GG", "competitive", [13, 5], "ancient", "2026-08-25T19:10:00Z", [0, 1, 0, 1, 1, 0, 2, 0]),
];
