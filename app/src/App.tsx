import { useEffect, useState } from "react";
import { Film } from "lucide-react";
import type { MatchRecord } from "./domain";
import { matchRepository } from "./data/matchRepository";
import { EmptySection } from "./components/EmptySection";
import { Sidebar, type Page } from "./components/Sidebar";
import { GameDetailPage } from "./pages/GameDetailPage";
import { GamesPage } from "./pages/GamesPage";
import { SettingsPage } from "./pages/SettingsPage";

function App() {
  const [page, setPage] = useState<Page>("games");
  const [matches, setMatches] = useState<MatchRecord[]>([]);
  const [selectedMatch, setSelectedMatch] = useState<MatchRecord | null>(null);

  useEffect(() => {
    void matchRepository.list().then(setMatches);
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
    content = <GamesPage matches={matches} onOpenMatch={openMatch} />;
  } else if (page === "montages") {
    content = (
      <div className="page centered-page">
        <EmptySection
          icon={Film}
          eyebrow="Coming later"
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
      <Sidebar currentPage={page} onNavigate={navigate} />
      <main className="main-content">{content}</main>
    </div>
  );
}

export default App;

