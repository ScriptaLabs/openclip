import { useEffect, useMemo, useState } from "react";
import { Film } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MatchRecord } from "./domain";
import {
  getCs2Status,
  getRecordingStatus,
  getManualClips,
  getPendingDiagnostics,
  updateSettings,
  type PendingReport,
  getSettings,
  matchRepository,
  type Cs2Status,
  type RecordingStatus,
} from "./data/matchRepository";
import { buildClips, type ClipEntry, type ManualClipRecord } from "./data/clips";
import { buildOverview } from "./data/overview";
import { EmptySection } from "./components/EmptySection";
import { DiagnosticsBanner } from "./components/DiagnosticsBanner";
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
  const [manualClips, setManualClips] = useState<ManualClipRecord[]>([]);
  const [report, setReport] = useState<PendingReport | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Resolves to whether the data loaded, so callers can retry a failed load.
  const refresh = useMemo(() => async (isActive: () => boolean = () => true): Promise<boolean> => {
    try {
      const [nextMatches, nextStatus, nextRecordingStatus, settings, nextManualClips] = await Promise.all([
        matchRepository.list(),
        getCs2Status(),
        getRecordingStatus(),
        getSettings(),
        getManualClips(),
      ]);
      if (!isActive()) return true;
      setMatches(nextMatches);
      setManualClips(nextManualClips);
      setStatus(nextStatus);
      setRecordingStatus(nextRecordingStatus);
      setSelectedMatch((current) => current ? nextMatches.find((item) => item.id === current.id) ?? null : null);
      document.documentElement.dataset.accent = settings.accent;
      setLoadError(null);
      return true;
    } catch (error) {
      if (isActive()) setLoadError(error instanceof Error ? error.message : String(error));
      return false;
    }
  }, []);

  useEffect(() => {
    let active = true;
    const isActive = () => active;
    // The desktop host can still be starting when the window first asks for data. A failed
    // load is retried with a growing delay instead of leaving its error on screen.
    let attempts = 0;
    let retry: number | undefined;
    const load = async () => {
      const loaded = await refresh(isActive);
      if (!loaded && isActive() && attempts < 12) {
        attempts += 1;
        retry = window.setTimeout(() => void load(), Math.min(1_000 + attempts * 500, 5_000));
      }
    };
    void load();
    const interval = window.setInterval(() => {
      void getCs2Status().then(setStatus).catch(() => {});
      void getRecordingStatus().then(setRecordingStatus).catch(() => {});
    }, 1_000);
    const unlisten: Array<() => void> = [];
    if (isTauri()) {
      void listen("gsi_update", () => void refresh(isActive)).then((stop) => { unlisten.push(stop); });
      // The capture confirmation itself is shown by the overlay window, not in this window.
      void listen("automatic_moment_finalized", () => void refresh(isActive)).then((stop) => { unlisten.push(stop); });
      void listen("replay_saved", () => void refresh(isActive)).then((stop) => { unlisten.push(stop); });
      void listen<PendingReport>("diagnostics_pending", (event) => setReport(event.payload)).then((stop) => { unlisten.push(stop); });
      void getPendingDiagnostics().then(setReport).catch(() => {});
    } else if (new URLSearchParams(window.location.search).has("diagnostics")) {
      // Browser-only design preview of the post-match problem report.
      setReport({
        id: "preview",
        incidentCount: 3,
        payload: {
          app_version: "0.1.10",
          os: "Windows 11 25H2 build 26200",
          gpus: [{ name: "AMD Radeon RX 6700 XT", vendor: "AMD", dedicatedMemoryMb: 12272, driverVersion: "32.0.21.1002", software: false }],
          encoder: "AMD H.264 Encoder MFT",
          settings: { videoEncoder: "auto", quality: "high", framesPerSecond: 60 },
          kinds: ["encoder_failed", "clip_save_failed"],
          fingerprint: "3fa9c1d2b7e04a18",
          summary: "AMD H.264 Encoder MFT returned no video after 180 pictures (+2 more)",
          log: "2026-11-08T20:02:00Z  WARN openclip_capture::windows: encoder failed error=AMD H.264 Encoder MFT returned no video after 180 pictures",
          user_note: null,
        },
      });
    }
    return () => {
      active = false;
      window.clearTimeout(retry);
      window.clearInterval(interval);
      unlisten.forEach((stop) => stop());
    };
  }, [refresh]);

  const clips = useMemo(() => buildClips(matches, manualClips), [matches, manualClips]);
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
    content = <SettingsPage recordingStatus={recordingStatus} />;
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
        {report && (
          <DiagnosticsBanner
            onDone={() => setReport(null)}
            onNeverAsk={() => void getSettings().then((current) => updateSettings({ ...current, diagnosticsPrompt: false }))}
            report={report}
          />
        )}
        <div className="page-transition" key={playing ? `clip-${playing.id}` : selectedMatch ? `match-${selectedMatch.id}` : page}>
          {content}
        </div>
      </main>
    </div>
  );
}

export default App;
