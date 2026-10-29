import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import OverlayApp, { type OverlayPayload } from "./overlay/OverlayApp";
import "@fontsource-variable/geist";
import "./styles.css";
import "./styles/app.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("OpenClip root element is missing");
}

// The overlay window loads this same bundle; it is told apart by its window label.
const isOverlayWindow = isTauri() && getCurrentWindow().label === "overlay";

// `npm run dev` with `?overlay=captured|partial|failed` previews the overlay in a browser.
function overlayDemo(): OverlayPayload | undefined {
  const params = new URLSearchParams(window.location.search);
  const status = params.get("overlay");
  if (isTauri() || (status !== "captured" && status !== "partial" && status !== "failed")) return undefined;
  const kill = (weapon: string, headshot: boolean) => ({
    id: crypto.randomUUID(), matchId: "demo", roundId: null, sourceGame: "counter_strike2" as const,
    eventType: "kill" as const, occurredAt: new Date().toISOString(), matchOffsetMs: 0,
    metadata: { weapon, headshot },
  });
  return {
    moment: {
      id: "demo-moment",
      classification: "quad_kill",
      events: [kill("weapon_ak47", true), kill("weapon_ak47", false), kill("weapon_ak47", true), kill("weapon_ak47", false)],
      clipPath: params.get("clip"),
    },
    status,
    missingMs: status === "partial" ? 4000 : 0,
    clipPreview: params.get("clip") !== null && status !== "failed",
    overlayPosition: params.get("side") === "right" ? "top_right" : "top_left",
    postRollSeconds: 5,
    soundsEnabled: false,
    soundVolume: 0,
  };
}

const demo = overlayDemo();
if (isOverlayWindow || demo) {
  document.documentElement.classList.add("overlay-root");
}

createRoot(root).render(
  <StrictMode>{isOverlayWindow || demo ? <OverlayApp demo={demo} /> : <App />}</StrictMode>,
);
