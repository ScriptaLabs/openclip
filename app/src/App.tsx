import { useEffect, useState } from "react";
import { Film } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MatchRecord } from "./domain";
import { getCs2Status, matchRepository, type Cs2Status } from "./data/matchRepository";
import { EmptySection } from "./components/EmptySection";
import { Sidebar, type Page } from "./components/Sidebar";
import { GameDetailPage } from "./pages/GameDetailPage";
import { GamesPage } from "./pages/GamesPage";
import { SettingsPage } from "./pages/SettingsPage";

function App() {
  const [page, setPage] = useState<Page>("games");
  const [matches, setMatches] = useState<MatchRecord[]>([]);
  const [selectedMatch, setSelectedMatch] = useState<MatchRecord | null>(null);
  const [status, setStatus] = useState<Cs2Status | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try {
        const [nextMatches, nextStatus] = await Promise.all([matchRepository.list(), getCs2Status()]);
        if (active) {
          setMatches(nextMatches);
          setStatus(nextStatus);
          setLoadError(null);
        }
      } catch (error) {
        if (active) setLoadError(error instanceof Error ? error.message : String(error));
      }
    };
    void refresh();
    const interval = window.setInterval(() => void getCs2Status().then(setStatus).catch(() => {}), 3_000);
    let unlisten: (() => void) | undefined;
    if (isTauri()) {
      void listen("gsi_update", () => void refresh()).then((stop) => { unlisten = stop; });
    }
    return () => {
      active = false;
      window.clearInterval(interval);
      unlisten?.();
    };
  }, []);

  const navigate = (nextPage: Page) => {
    setSelectedMatch(null);
    setPage(nextPage);
  };

  const openMatch = (id: string) => {
    void matchRepository.get(id).then(setSelectedMatch);
  };

  let content;
  if (selectedMatch) {
    content = <GameDetailPage match={selectedMatch} onBack={() => setSelectedMatch(null)} />;
  } else if (page === "games") {
    content = <GamesPage error={loadError} matches={matches} onOpenMatch={openMatch} />;
  } else if (page === "montages") {
    content = (
      <div className="page centered-page">
        <EmptySection
          icon={Film}
          title="Montages"
          body="Once match capture is reliable, your best moments will become polished videos here."
        />
      </div>
    );
  } else {
    content = <SettingsPage />;
  }

  return (
    <div className="app-shell">
      <Sidebar currentPage={page} onNavigate={navigate} status={status} />
      <main className="main-content">{content}</main>
    </div>
  );
}

export default App;
