import { invoke, isTauri } from "@tauri-apps/api/core";
import type { MatchRecord, Moment } from "../domain";
import { momentTitle } from "../domain";
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

export interface RecordingStatus {
  state: "idle" | "waiting_for_game" | "initializing" | "recording" | "saving_replay" | "error";
  game: "counter_strike2" | null;
  message: string | null;
  encoder: string | null;
  captureWidth: number | null;
  captureHeight: number | null;
  captureFps: number | null;
  bufferedSeconds: number;
}

export interface RecordingSettings {
  quality: "performance" | "high" | "ultra";
  resolution: "game_resolution" | "p1080" | "p1440";
  framesPerSecond: number;
  replayBufferSeconds: number;
  preRollSeconds: number;
  postRollSeconds: number;
  captureKills: boolean;
  captureHeadshots: boolean;
  captureMultiKills: boolean;
  captureGameAudio: boolean;
  captureMicrophone: boolean;
  microphoneDeviceId: string | null;
  storagePath: string;
  accent: "mint" | "blue" | "purple" | "neutral";
  showClipNotifications: boolean;
  manualReplayShortcut: string;
  watermarkEnabled: boolean;
  overlayPosition: "top_left" | "top_right";
  overlayClipPreview: boolean;
  soundsEnabled: boolean;
  /** 0 to 100 */
  soundVolume: number;
}

export interface AudioDevice {
  id: string;
  name: string;
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
        title: momentTitle(moment),
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

export async function getRecordingStatus(): Promise<RecordingStatus> {
  if (!isTauri()) {
    return {
      state: "waiting_for_game",
      game: "counter_strike2",
      message: "Browser preview · capture runs in the desktop app",
      encoder: null,
      captureWidth: null,
      captureHeight: null,
      captureFps: null,
      bufferedSeconds: 0,
    };
  }
  return invoke<RecordingStatus>("get_recording_status");
}

export async function saveReplay(): Promise<void> {
  await invoke("save_replay");
}

const browserSettings: RecordingSettings = {
  quality: "high",
  resolution: "game_resolution",
  framesPerSecond: 60,
  replayBufferSeconds: 45,
  preRollSeconds: 10,
  postRollSeconds: 5,
  captureKills: true,
  captureHeadshots: true,
  captureMultiKills: true,
  captureGameAudio: true,
  captureMicrophone: false,
  microphoneDeviceId: null,
  storagePath: "OpenClip/Clips",
  accent: "mint",
  showClipNotifications: true,
  manualReplayShortcut: "F8",
  watermarkEnabled: false,
  overlayPosition: "top_left",
  overlayClipPreview: true,
  soundsEnabled: true,
  soundVolume: 60,
};

export async function getSettings(): Promise<RecordingSettings> {
  return isTauri() ? invoke<RecordingSettings>("get_settings") : browserSettings;
}

export async function updateSettings(settings: RecordingSettings): Promise<void> {
  if (isTauri()) await invoke("update_settings", { settings });
}

export async function previewOverlay(status: "captured" | "partial" | "failed"): Promise<void> {
  if (isTauri()) await invoke("preview_overlay", { status });
}

export async function getMicrophoneDevices(): Promise<AudioDevice[]> {
  return isTauri() ? invoke<AudioDevice[]>("get_microphone_devices") : [];
}

export async function setStoragePath(path: string): Promise<RecordingSettings> {
  return invoke<RecordingSettings>("set_storage_path", { path });
}

export async function openMomentClip(matchId: string, momentId: string): Promise<void> {
  await invoke("open_moment_clip", { matchId, momentId });
}

export async function exportInterestingMoments(matchId: string, destination: string): Promise<string[]> {
  return invoke<string[]>("export_interesting_moments", { matchId, destination });
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
