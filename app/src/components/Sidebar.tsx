import { Clapperboard, Gamepad2, Settings } from "lucide-react";
import { BrandMark } from "./BrandMark";

export type Page = "games" | "montages" | "settings";

interface SidebarProps {
  currentPage: Page;
  onNavigate: (page: Page) => void;
}

const navigation = [
  { id: "games" as const, label: "Games", icon: Gamepad2 },
  { id: "montages" as const, label: "Montages", icon: Clapperboard },
];

export function Sidebar({ currentPage, onNavigate }: SidebarProps) {
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
          <span><i /> Waiting for game</span>
        </div>
      </div>
    </aside>
  );
}

