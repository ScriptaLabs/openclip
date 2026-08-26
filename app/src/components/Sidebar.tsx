import { Clapperboard, Gamepad2, Settings } from "lucide-react";
import { BrandMark } from "./BrandMark";
import type { Cs2Status } from "../data/matchRepository";

export type Page = "games" | "montages" | "settings";

interface SidebarProps {
  currentPage: Page;
  onNavigate: (page: Page) => void;
  status: Cs2Status | null;
}

const navigation = [
  { id: "games" as const, label: "Games", icon: Gamepad2 },
  { id: "montages" as const, label: "Montages", icon: Clapperboard },
];

export function Sidebar({ currentPage, onNavigate, status }: SidebarProps) {
  const statusLabel = status?.state === "match_in_progress"
    ? "Match in progress"
    : status?.state === "connected" ? "CS2 connected"
      : status?.state === "error" ? "Listener error" : "Waiting for game";
  return (
    <aside className="sidebar">
      <div className="brand-row">
        <BrandMark />
        <span>OpenClip</span>
      </div>

      <nav className="primary-nav" aria-label="Primary navigation">
        {navigation.map(({ id, label, icon: Icon }) => (
          <button
            className={`nav-item ${currentPage === id ? "active" : ""}`}
            key={id}
            onClick={() => onNavigate(id)}
            type="button"
          >
            <Icon size={18} strokeWidth={1.8} />
            {label}
          </button>
        ))}
      </nav>

      <div className="sidebar-spacer" />
      <button
        className={`nav-item ${currentPage === "settings" ? "active" : ""}`}
        onClick={() => onNavigate("settings")}
        type="button"
      >
        <Settings size={18} strokeWidth={1.8} />
        Settings
      </button>

      <div className="recording-card">
        <div className="game-avatar">CS2</div>
        <div>
          <strong>Counter-Strike 2</strong>
          <span className={`connection-${status?.state ?? "waiting_for_game"}`}><i /> {statusLabel}</span>
        </div>
      </div>
    </aside>
  );
}
