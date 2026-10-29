import { Clapperboard, Gamepad2, LayoutDashboard, LayoutGrid, SlidersHorizontal, type LucideIcon } from "lucide-react";
import type { Cs2Status, RecordingStatus } from "../data/matchRepository";
import logo from "../../assets/branding/logo.png";
import { UpdateIndicator, VersionLabel } from "./UpdateIndicator";

export type Page = "overview" | "clips" | "matches" | "montages" | "settings";

interface SidebarProps {
  currentPage: Page;
  onNavigate: (page: Page) => void;
  status: Cs2Status | null;
  recordingStatus: RecordingStatus | null;
  clipCount: number;
}

const navigation: Array<{ id: Page; label: string; icon: LucideIcon }> = [
  { id: "overview", label: "Overview", icon: LayoutDashboard },
  { id: "clips", label: "Clips", icon: LayoutGrid },
  { id: "matches", label: "Matches", icon: Gamepad2 },
  { id: "montages", label: "Montages", icon: Clapperboard },
];

function statusFor(status: Cs2Status | null, recording: RecordingStatus | null) {
  if (recording?.state === "recording" || recording?.state === "saving_replay") {
    return { tone: "live", title: "Recording CS2", detail: recording.bufferedSeconds > 0 ? `${Math.round(recording.bufferedSeconds)}s buffered` : "Replay buffer active" };
  }
  if (recording?.state === "error" || status?.state === "error") {
    return { tone: "error", title: "Needs attention", detail: recording?.message ?? status?.message ?? "Check Settings" };
  }
  if (status?.state === "match_in_progress") return { tone: "live", title: "Match in progress", detail: status.map ?? "Counter-Strike 2" };
  if (status?.state === "connected") return { tone: "live", title: "CS2 connected", detail: "Waiting for a match" };
  return { tone: "idle", title: "Waiting for CS2", detail: recording?.message ?? "Launch the game to start" };
}

export function Sidebar({ currentPage, onNavigate, status, recordingStatus, clipCount }: SidebarProps) {
  const state = statusFor(status, recordingStatus);
  const item = (id: Page, label: string, Icon: LucideIcon, extra?: string) => (
    <button
      aria-current={currentPage === id ? "page" : undefined}
      className={`oc-nav-item ${currentPage === id ? "active" : ""}`}
      key={id}
      onClick={() => onNavigate(id)}
      type="button"
    >
      <Icon size={18} strokeWidth={2} />
      <span>{label}</span>
      {id === "clips" && clipCount > 0 && <span className="oc-nav-count">{clipCount}</span>}
      {id === "montages" && <span className="oc-soon">Soon</span>}
      {extra}
    </button>
  );
  return (
    <aside className="oc-sidebar">
      <div className="oc-brand">
        <img alt="" src={logo} />
        <span>OpenClip</span>
      </div>
      <nav aria-label="Primary" style={{ display: "grid", gap: 6 }}>
        {navigation.map(({ id, label, icon }) => item(id, label, icon))}
      </nav>
      <div className="oc-grow" />
      {item("settings", "Settings", SlidersHorizontal)}
      <UpdateIndicator />
      <div className={`oc-status ${state.tone}`}>
        <i />
        <div>
          <strong>{state.title}</strong>
          <span>{state.detail}</span>
        </div>
      </div>
      <VersionLabel />
    </aside>
  );
}
