import { useEffect, useMemo, useState } from "react";
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
import { buildClips, type ClipEntry } from "./data/clips";
import { buildOverview } from "./data/overview";
import { EmptySection } from "./components/EmptySection";
import { Sidebar, type Page } from "./components/Sidebar";
import { ClipsPage } from "./pages/ClipsPage";
import { MatchDetailPage } from "./pages/MatchDetailPage";
import { MatchesPage } from "./pages/MatchesPage";
import { OverviewPage } from "./pages/OverviewPage";
import { PlayerPage } from "./pages/PlayerPage";
import { SettingsPage } from "./pages/SettingsPage";

function App() {
  const [page, setPage] = useState<Page>("overview");
  const [matches, setMatches] = useState<MatchRecord[]>([]);
  const [selectedMatch, setSelectedMatch] = useState<MatchRecord | null>(null);
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [status, setStatus] = useState<Cs2Status | null>(null);
  const [recordingStatus, setRecordingStatus] = useState<RecordingStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const refresh = useMemo(() => async (isActive: () => boolean = () => true) => {
    try {
      const [nextMatches, nextStatus, nextRecordingStatus, settings] = await Promise.all([
        matchRepository.list(),
        getCs2Status(),
        getRecordingStatus(),
        getSettings(),
      ]);
      if (!isActive()) return;
      setMatches(nextMatches);
      setStatus(nextStatus);
      setRecordingStatus(nextRecordingStatus);
      setSelectedMatch((current) => current ? nextMatches.find((item) => item.id === current.id) ?? null : null);
      document.documentElement.dataset.accent = settings.accent;
      setLoadError(null);
    } catch (error) {
      if (isActive()) setLoadError(error instanceof Error ? error.message : String(error));
    }
  }, []);

  useEffect(() => {
    let active = true;
    const isActive = () => active;
    void refresh(isActive);
    const interval = window.setInterval(() => {
      void getCs2Status().then(setStatus).catch(() => {});
      void getRecordingStatus().then(setRecordingStatus).catch(() => {});
    }, 1_000);
    const unlisten: Array<() => void> = [];
    if (isTauri()) {
      void listen("gsi_update", () => void refresh(isActive)).then((stop) => { unlisten.push(stop); });
      // The capture confirmation itself is shown by the overlay window, not in this window.
      void listen("automatic_moment_finalized", () => void refresh(isActive)).then((stop) => { unlisten.push(stop); });
    }
    return () => {
      active = false;
      window.clearInterval(interval);
      unlisten.forEach((stop) => stop());
    };
  }, [refresh]);

  const clips = useMemo(() => buildClips(matches), [matches]);
  const overview = useMemo(() => buildOverview(matches, clips), [matches, clips]);
  const playing = playingId ? clips.find((clip) => clip.id === playingId) ?? null : null;

  const navigate = (nextPage: Page) => {
    setSelectedMatch(null);
    setPlayingId(null);
    setPage(nextPage);
  };

  const openMatch = (id: string) => {
    setPlayingId(null);
    setPage("matches");
    void matchRepository.get(id).then(setSelectedMatch);
  };

  const openClip = (clip: ClipEntry) => setPlayingId(clip.id);

  let content;
  if (playing) {
    content = (
      <PlayerPage
        clip={playing}
        clips={clips}
        onBack={() => setPlayingId(null)}
        onChanged={() => void refresh()}
        onOpenMatch={openMatch}
        onSelect={openClip}
      />
    );
  } else if (selectedMatch) {
    content = <MatchDetailPage clips={clips} match={selectedMatch} onBack={() => setSelectedMatch(null)} onOpenClip={openClip} />;
  } else if (page === "overview") {
    content = (
      <OverviewPage
        error={loadError}
        onOpenClip={openClip}
        onOpenMatch={openMatch}
        onSeeAll={() => navigate("clips")}
        overview={overview}
      />
    );
  } else if (page === "clips") {
    content = <ClipsPage clips={clips} error={loadError} onOpenClip={openClip} />;
  } else if (page === "matches") {
    content = <MatchesPage clips={clips} error={loadError} matches={matches} onOpenMatch={openMatch} />;
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
    <div className="oc-shell">
      <Sidebar
        clipCount={clips.filter((clip) => clip.hasClip).length}
        currentPage={page}
        onNavigate={navigate}
        recordingStatus={recordingStatus}
        status={status}
      />
      <main className="main-content">
        <div className="page-transition" key={playing ? `clip-${playing.id}` : selectedMatch ? `match-${selectedMatch.id}` : page}>
          {content}
        </div>
      </main>
    </div>
  );
}

export default App;
