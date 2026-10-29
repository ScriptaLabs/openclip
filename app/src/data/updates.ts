import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type UpdatePhase = "idle" | "checking" | "available" | "downloading" | "ready" | "installing";

export interface UpdateStatus {
  phase: UpdatePhase;
  currentVersion: string;
  availableVersion: string | null;
  notes: string | null;
  /** 0 to 1 while downloading. */
  progress: number;
  lastChecked: string | null;
  error: string | null;
}

const browserStatus: UpdateStatus = {
  phase: "idle",
  currentVersion: "0.0.0-preview",
  availableVersion: null,
  notes: null,
  progress: 0,
  lastChecked: null,
  error: null,
};

export async function getUpdateStatus(): Promise<UpdateStatus> {
  return isTauri() ? invoke<UpdateStatus>("get_update_status") : browserStatus;
}

export async function checkForUpdate(): Promise<UpdateStatus> {
  return isTauri() ? invoke<UpdateStatus>("check_for_update") : browserStatus;
}

export async function installUpdateNow(): Promise<void> {
  if (isTauri()) await invoke("update_now");
}

export async function getLaunchAtStartup(): Promise<boolean> {
  return isTauri() ? invoke<boolean>("get_launch_at_startup") : false;
}

export async function setLaunchAtStartup(enabled: boolean): Promise<void> {
  if (isTauri()) await invoke("set_launch_at_startup", { enabled });
}

/** The live update status, pushed by the desktop host. */
export function useUpdateStatus(): UpdateStatus {
  const [status, setStatus] = useState<UpdateStatus>(browserStatus);
  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;
    void getUpdateStatus().then((initial) => { if (active) setStatus(initial); }).catch(() => {});
    if (isTauri()) {
      void listen<UpdateStatus>("update_status", (event) => setStatus(event.payload)).then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      });
    }
    return () => {
      active = false;
      stop?.();
    };
  }, []);
  return status;
}
