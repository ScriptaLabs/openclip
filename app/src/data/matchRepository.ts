import type { MatchRecord } from "../domain";
import { developmentMatches } from "./developmentFixtures";

export interface MatchRepository {
  list(): Promise<MatchRecord[]>;
  get(id: string): Promise<MatchRecord | null>;
}

/**
 * Milestone 1 data source. The pages depend only on MatchRepository; Milestone 3 can replace this
 * with a Tauri IPC implementation once GSI has produced real persisted matches.
 */
class FixtureMatchRepository implements MatchRepository {
  async list(): Promise<MatchRecord[]> {
    return developmentMatches;
  }

  async get(id: string): Promise<MatchRecord | null> {
    return developmentMatches.find((match) => match.id === id) ?? null;
  }
}

export const matchRepository: MatchRepository = new FixtureMatchRepository();

