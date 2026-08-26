import { Clapperboard, Gamepad2, Settings } from "lucide-react";
import { BrandMark } from "./BrandMark";
import type { Cs2Status } from "../data/matchRepository";
import type { RecordingStatus } from "../data/matchRepository";
import banner from "../../assets/branding/banner.png";
import cs2Artwork from "../../assets/games/cs2.jpg";

export type Page = "games" | "montages" | "settings";

interface SidebarProps {
  currentPage: Page;
  onNavigate: (page: Page) => void;
  status: Cs2Status | null;
  recordingStatus: RecordingStatus | null;
}

const navigation = [
  { id: "games" as const, label: "Games", icon: Gamepad2 },
  { id: "montages" as const, label: "Montages", icon: Clapperboard },
];

export function Sidebar({ currentPage, onNavigate, status, recordingStatus }: SidebarProps) {
  const gsiStatusLabel = status?.state === "match_in_progress"
    ? "Match in progress"
    : status?.state === "connected" ? "CS2 connected"
      : status?.state === "error" ? "Listener error" : "Waiting for game";
  return (
    <aside className="sidebar">
      <div className="brand-row">
        <BrandMark />
        <img className="brand-banner" src={banner} alt="OpenClip" />
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
        <img className="game-avatar" src={cs2Artwork} alt="" />
        <div>
          <strong>Counter-Strike 2</strong>
          <span className={`connection-${recordingStatus?.state ?? status?.state ?? "waiting_for_game"}`}>
            <i /> {recordingStatus?.message ?? gsiStatusLabel}
          </span>
        </div>
      </div>
    </aside>
  );
}
