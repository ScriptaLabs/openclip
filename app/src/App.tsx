import { useEffect, useState } from "react";
import { Film } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MatchRecord } from "./domain";
import {
  getCs2Status,
  getRecordingStatus,
  getSettings,
  matchRepository,
  type Cs2Status,
  type RecordingStatus,
} from "./data/matchRepository";
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
  const [recordingStatus, setRecordingStatus] = useState<RecordingStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [clipNotice, setClipNotice] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try {
        const [nextMatches, nextStatus, nextRecordingStatus, settings] = await Promise.all([
          matchRepository.list(),
          getCs2Status(),
          getRecordingStatus(),
          getSettings(),
        ]);
        if (active) {
          setMatches(nextMatches);
          setStatus(nextStatus);
          setRecordingStatus(nextRecordingStatus);
          setSelectedMatch((current) => current ? nextMatches.find((item) => item.id === current.id) ?? null : null);
          document.documentElement.dataset.accent = settings.accent;
          setLoadError(null);
        }
      } catch (error) {
        if (active) setLoadError(error instanceof Error ? error.message : String(error));
      }
    };
    void refresh();
    const interval = window.setInterval(() => {
      void getCs2Status().then(setStatus).catch(() => {});
      void getRecordingStatus().then(setRecordingStatus).catch(() => {});
    }, 1_000);
    const unlisten: Array<() => void> = [];
    if (isTauri()) {
      void listen("gsi_update", () => void refresh()).then((stop) => { unlisten.push(stop); });
      void listen<{ moment: { classification: string }; captured: boolean }>("automatic_moment_finalized", async (event) => {
        await refresh();
        const settings = await getSettings();
        if (event.payload.captured && settings.showClipNotifications) {
          const label = event.payload.moment.classification.split("_").map((part) => part[0]!.toUpperCase() + part.slice(1)).join(" ");
          setClipNotice(`Clip captured · ${label}`);
          window.setTimeout(() => setClipNotice(null), 4_000);
        }
      }).then((stop) => { unlisten.push(stop); });
    }
    return () => {
      active = false;
      window.clearInterval(interval);
      unlisten.forEach((stop) => stop());
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
      <Sidebar
        currentPage={page}
        onNavigate={navigate}
        status={status}
        recordingStatus={recordingStatus}
      />
      <main className="main-content">{content}</main>
      {clipNotice && <div className="clip-notice" role="status">{clipNotice}</div>}
    </div>
  );
}

export default App;
