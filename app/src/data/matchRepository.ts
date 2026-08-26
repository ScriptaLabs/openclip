import { invoke, isTauri } from "@tauri-apps/api/core";
import type { MatchRecord, Moment } from "../domain";
import { developmentMatches } from "./developmentFixtures";

type PersistedMatch = Omit<MatchRecord, "thumbnailTone" | "moments"> & {
  moments: Array<Omit<Moment, "title" | "weapon" | "thumbnailTone">>;
};

export interface Cs2Status {
  state: "waiting_for_game" | "connected" | "match_in_progress" | "error";
  map: string | null;
  activeMatchId: string | null;
  lastUpdateAt: string | null;
  message: string;
}

export interface Cs2SetupStatus {
  found: boolean;
  configured: boolean;
  install: { installDir: string; cfgDir: string } | null;
  message: string;
}

export interface MatchRepository {
  list(): Promise<MatchRecord[]>;
  get(id: string): Promise<MatchRecord | null>;
}

function toneForMap(map: string | null): MatchRecord["thumbnailTone"] {
  if (map?.includes("inferno")) return "inferno";
  if (map?.includes("ancient")) return "ancient";
  return "mirage";
}

function enrichMatch(match: PersistedMatch): MatchRecord {
  return {
    ...match,
    thumbnailTone: toneForMap(match.map),
    moments: match.moments.map((moment, index) => {
      const weapon = moment.events.find((event) => typeof event.metadata.weapon === "string")
        ?.metadata.weapon as string | undefined;
      return {
        ...moment,
        title: moment.classification === "headshot" ? "Headshot" : "Highlight",
        weapon: weapon?.replace("weapon_", "") ?? "Unknown weapon",
        thumbnailTone: (["amber", "cyan", "violet", "green"] as const)[index % 4]!,
      };
    }),
  };
}

class TauriMatchRepository implements MatchRepository {
  async list(): Promise<MatchRecord[]> {
    return (await invoke<PersistedMatch[]>("get_games")).map(enrichMatch);
  }

  async get(id: string): Promise<MatchRecord | null> {
    const match = await invoke<PersistedMatch | null>("get_game", { id });
    return match ? enrichMatch(match) : null;
  }
}

class FixtureMatchRepository implements MatchRepository {
  async list(): Promise<MatchRecord[]> {
    return developmentMatches;
  }

  async get(id: string): Promise<MatchRecord | null> {
    return developmentMatches.find((match) => match.id === id) ?? null;
  }
}

export const matchRepository: MatchRepository = isTauri()
  ? new TauriMatchRepository()
  : new FixtureMatchRepository();

export async function getCs2Status(): Promise<Cs2Status> {
  if (!isTauri()) {
    return {
      state: "waiting_for_game",
      map: null,
      activeMatchId: null,
      lastUpdateAt: null,
      message: "Browser preview · fixture data",
    };
  }
  return invoke<Cs2Status>("get_cs2_status");
}

export async function getCs2SetupStatus(): Promise<Cs2SetupStatus> {
  if (!isTauri()) {
    return { found: false, configured: false, install: null, message: "Available in the desktop app" };
  }
  return invoke<Cs2SetupStatus>("get_cs2_setup_status");
}

export async function installCs2GsiConfig(): Promise<void> {
  await invoke("install_cs2_gsi_config");
}
