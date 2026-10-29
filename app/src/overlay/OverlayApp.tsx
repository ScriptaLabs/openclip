import { useEffect, useRef, useState } from "react";
import { Check, TriangleAlert, X } from "lucide-react";
import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BrandMark } from "../components/BrandMark";
import { dominantWeapon, killCount, momentTitle, type Moment } from "../domain";
import { getSettings } from "../data/matchRepository";
import { playSound } from "./sounds";
import "./overlay.css";

export type CaptureStatus = "captured" | "partial" | "failed";

export interface OverlayPayload {
  moment: Pick<Moment, "classification" | "events" | "clipPath" | "id">;
  status: CaptureStatus;
  missingMs: number;
  clipPreview: boolean;
  overlayPosition: "top_left" | "top_right";
  postRollSeconds: number;
  soundsEnabled: boolean;
  soundVolume: number;
}

type Phase = "enter" | "exit";

const ENTER_MS = 460;
const EXIT_MS = 360;
const PREVIEW_OPEN_MS = 520;
const PREVIEW_PLAY_MS = 2_800;
const HOLD_MS = 3_200;
const FAILED_HOLD_MS = 4_400;
/** How long to wait for the clip to become playable before showing the card without it. */
const PREVIEW_WAIT_MS = 1_500;
/** Seconds of the clip shown before its final kill. */
const PREVIEW_LEAD_SECONDS = 2.2;

const statusCopy: Record<CaptureStatus, string> = {
  captured: "Clip captured",
  partial: "Partial clip saved",
  failed: "Couldn't capture this moment",
};

function detail(payload: OverlayPayload): string {
  const kills = killCount(payload.moment);
  const parts: string[] = [];
  if (kills > 1) parts.push(`${kills} kills`);
  const weapon = dominantWeapon(payload.moment);
  if (weapon) parts.push(weapon);
  if (payload.status === "partial" && payload.missingMs >= 500) {
    parts.push(`first ${Math.round(payload.missingMs / 1000)}s missing`);
  }
  return parts.join(" · ");
}

interface OverlayAppProps {
  /** Browser-only design preview: feeds the overlay a notification instead of Tauri events. */
  demo?: OverlayPayload;
}

export default function OverlayApp({ demo }: OverlayAppProps) {
  const [item, setItem] = useState<OverlayPayload | null>(null);
  const [phase, setPhase] = useState<Phase>("enter");
  const [previewOpen, setPreviewOpen] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);
  const controller = useRef<{ push: (payload: OverlayPayload) => void; ready: () => void } | null>(null);

  useEffect(() => {
    const queue: OverlayPayload[] = [];
    const timers: number[] = [];
    let current: OverlayPayload | null = null;
    let enterDone = false;
    let videoReady = false;
    let exiting = false;
    let previewStarted = false;
    let disposed = false;

    const later = (callback: () => void, ms: number) => {
      timers.push(window.setTimeout(() => { if (!disposed) callback(); }, ms));
    };
    const clearTimers = () => { timers.splice(0).forEach((id) => window.clearTimeout(id)); };

    const finish = () => {
      clearTimers();
      const next = queue.shift();
      if (next) {
        begin(next);
        return;
      }
      current = null;
      setItem(null);
      setPreviewOpen(false);
      if (isTauri()) void invoke("hide_overlay");
    };

    const beginExit = () => {
      if (exiting) return;
      exiting = true;
      setPhase("exit");
      later(finish, EXIT_MS);
    };

    const openPreview = () => {
      const video = videoRef.current;
      if (!video || exiting || previewStarted) return;
      previewStarted = true;
      setPreviewOpen(true);
      void video.play().catch(() => {});
      later(beginExit, PREVIEW_OPEN_MS + PREVIEW_PLAY_MS);
    };

    const afterEnter = () => {
      enterDone = true;
      if (!current) return;
      if (!current.clipPreview) {
        later(beginExit, current.status === "failed" ? FAILED_HOLD_MS : HOLD_MS);
      } else if (videoReady) {
        openPreview();
      } else {
        // The clip never became playable in time: confirm the capture without it.
        later(() => {
          if (!videoReady) beginExit();
        }, PREVIEW_WAIT_MS);
      }
    };

    function begin(payload: OverlayPayload) {
      clearTimers();
      current = payload;
      enterDone = false;
      videoReady = false;
      exiting = false;
      previewStarted = false;
      setItem(payload);
      setPhase("enter");
      setPreviewOpen(false);
      void getSettings().then((settings) => {
        document.documentElement.dataset.accent = settings.accent;
      }).catch(() => {});
      if (payload.soundsEnabled) {
        playSound(payload.status === "failed" ? "error" : "captured", payload.soundVolume);
      }
      later(afterEnter, ENTER_MS);
    }

    controller.current = {
      push: (payload) => {
        if (current) {
          queue.push(payload);
        } else {
          begin(payload);
        }
      },
      ready: () => {
        videoReady = true;
        if (enterDone && !exiting && current?.clipPreview) openPreview();
      },
    };

    let stopListening: (() => void) | undefined;
    if (demo) {
      begin(demo);
    } else if (isTauri()) {
      void listen<OverlayPayload>("overlay_show", (event) => controller.current?.push(event.payload)).then(
        (stop) => {
          if (disposed) stop();
          else stopListening = stop;
        },
      );
    }
    return () => {
      disposed = true;
      clearTimers();
      stopListening?.();
    };
  }, [demo]);

  if (!item) return <div className="ov-stage" />;

  const previewSrc = item.clipPreview && item.moment.clipPath ? clipUrl(item.moment.clipPath) : null;
  const subtitle = detail(item);
  const StatusIcon = item.status === "captured" ? Check : item.status === "partial" ? TriangleAlert : X;

  return (
    <div className="ov-stage" data-side={item.overlayPosition === "top_right" ? "right" : "left"}>
      <article
        className="ov-card"
        data-phase={phase}
        data-status={item.status}
        key={item.moment.id}
        role="status"
      >
        <header className="ov-head">
          <span className="ov-logo"><BrandMark /></span>
          <div className="ov-text">
            <span className="ov-status"><i><StatusIcon size={11} strokeWidth={3} /></i>{statusCopy[item.status]}</span>
            <strong>{momentTitle(item.moment)}</strong>
            {subtitle && <small>{subtitle}</small>}
          </div>
        </header>
        {previewSrc && (
          <div className="ov-preview" data-open={previewOpen}>
            <div className="ov-preview-clip">
              <video
                muted
                playsInline
                preload="auto"
                ref={videoRef}
                src={previewSrc}
                onCanPlay={() => controller.current?.ready()}
                onError={() => undefined}
                onLoadedMetadata={(event) => {
                  const video = event.currentTarget;
                  const start = video.duration - item.postRollSeconds - PREVIEW_LEAD_SECONDS;
                  video.currentTime = Number.isFinite(start) ? Math.max(0, start) : 0;
                }}
              />
              <span className="ov-progress" data-running={previewOpen}><i /></span>
            </div>
          </div>
        )}
      </article>
    </div>
  );
}

function clipUrl(path: string): string {
  if (!isTauri()) return path;
  return convertFileSrc(path);
}
