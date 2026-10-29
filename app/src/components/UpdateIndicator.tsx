import { useEffect, useState } from "react";
import { ArrowDownToLine, Check, RefreshCw } from "lucide-react";
import { installUpdateNow, useUpdateStatus } from "../data/updates";

const SEEN_KEY = "openclip.lastRunVersion";

/** Remembers the running version and reports the previous one when it changed (an update). */
function useJustUpdatedFrom(current: string): string | null {
  const [previous, setPrevious] = useState<string | null>(null);
  useEffect(() => {
    if (current === "0.0.0-preview") return;
    try {
      const last = window.localStorage.getItem(SEEN_KEY);
      if (last && last !== current) setPrevious(last);
      window.localStorage.setItem(SEEN_KEY, current);
    } catch {
      // Remembering is a nicety; the update itself does not depend on it.
    }
  }, [current]);
  useEffect(() => {
    if (!previous) return;
    const timer = window.setTimeout(() => setPrevious(null), 7_000);
    return () => window.clearTimeout(timer);
  }, [previous]);
  return previous;
}

/**
 * The only visible trace of the silent updater: nothing while idle, a thin progress line while
 * downloading, and one calm line when an update is waiting for a safe moment.
 */
export function UpdateIndicator() {
  const status = useUpdateStatus();
  const updatedFrom = useJustUpdatedFrom(status.currentVersion);
  const version = status.availableVersion;

  return (
    <>
      {status.phase === "downloading" && (
        <div className="oc-update" role="status">
          <ArrowDownToLine size={15} />
          <div>
            <strong>Updating to v{version}</strong>
            <span className="oc-update-bar"><i style={{ transform: `scaleX(${Math.max(status.progress, 0.04)})` }} /></span>
          </div>
        </div>
      )}
      {status.phase === "ready" && (
        <div className="oc-update ready" role="status">
          <Check size={15} />
          <div>
            <strong>v{version} is ready</strong>
            <span>Installs when your match ends</span>
          </div>
          <button onClick={() => void installUpdateNow()} type="button">Restart</button>
        </div>
      )}
      {status.phase === "available" && (
        <div className="oc-update" role="status">
          <RefreshCw size={15} />
          <div>
            <strong>v{version} available</strong>
            <span>Automatic updates are off</span>
          </div>
          <button onClick={() => void installUpdateNow()} type="button">Update</button>
        </div>
      )}
      {status.phase === "installing" && (
        <div className="oc-update" role="status">
          <RefreshCw className="spin" size={15} />
          <div><strong>Restarting…</strong></div>
        </div>
      )}
      {updatedFrom && (
        <div className="oc-toast" role="status">
          <Check size={16} />
          <div>
            <strong>OpenClip updated</strong>
            <span>You’re on v{status.currentVersion}</span>
          </div>
        </div>
      )}
    </>
  );
}

export function VersionLabel() {
  const { currentVersion } = useUpdateStatus();
  return <span className="oc-version">v{currentVersion}</span>;
}
