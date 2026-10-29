import { useMemo, useState } from "react";
import { ChevronLeft, Download } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { isTauri } from "@tauri-apps/api/core";
import { ClipCard } from "../components/ClipCard";
import { BestMoment, MatchRecap, RoundsChart, WeaponsCard } from "../components/MatchSections";
import type { ClipEntry } from "../data/clips";
import { exportInterestingMoments } from "../data/matchRepository";
import { buildMatchSummary } from "../data/overview";
import type { MatchRecord } from "../domain";

interface MatchDetailPageProps {
  match: MatchRecord;
  clips: ClipEntry[];
  onBack: () => void;
  onOpenClip: (clip: ClipEntry) => void;
}

export function MatchDetailPage({ match, clips, onBack, onOpenClip }: MatchDetailPageProps) {
  const summary = useMemo(() => buildMatchSummary(match, clips), [clips, match]);
  const [exporting, setExporting] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const playable = summary.clips.filter((clip) => clip.hasClip);
  const unavailable = summary.clips.length - playable.length;

  const exportMoments = async () => {
    setMessage(null);
    try {
      const destination = await open({ directory: true, multiple: false, title: "Export interesting moments" });
      if (!destination) return;
      setExporting(true);
      const paths = await exportInterestingMoments(match.id, destination);
      setMessage({ text: `${paths.length} ${paths.length === 1 ? "clip" : "clips"} exported`, error: false });
    } catch (error) {
      setMessage({ text: error instanceof Error ? error.message : String(error), error: true });
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="oc-page">
      <div className="oc-player-top">
        <button className="oc-btn" onClick={onBack} type="button"><ChevronLeft size={16} /> Matches</button>
        <button className="oc-btn primary" disabled={!isTauri() || playable.length === 0 || exporting} onClick={() => void exportMoments()} type="button">
          <Download size={16} /> {exporting ? "Exporting…" : "Export interesting moments"}
        </button>
      </div>

      {message && <div className={message.error ? "oc-error" : "oc-note"} role="status">{message.text}</div>}

      <div className="oc-row-layout">
        <MatchRecap summary={summary} />
        {summary.best && <BestMoment clip={summary.best} onOpen={onOpenClip} />}
      </div>
      <RoundsChart onOpenClip={onOpenClip} summary={summary} />
      <div className="oc-two">
        <WeaponsCard summary={summary} />
        <section className="oc-card">
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 16 }}>
            <h2 className="oc-h2">Captured moments</h2>
            <span className={`oc-health-badge ${unavailable > 0 ? "warn" : ""}`}><i />{unavailable > 0 ? "Some missed" : "All good"}</span>
          </div>
          <div className="oc-big-number">{playable.length} <span>of {summary.clips.length}</span></div>
          <p className="oc-sub" style={{ margin: "6px 0 18px", fontSize: 13.5 }}>moments from this match were saved as clips.</p>
          <div className="oc-split">
            <i className="ok" style={{ flex: Math.max(playable.length, 0.001) }} />
            {unavailable > 0 && <i className="miss" style={{ flex: unavailable }} />}
          </div>
          <div className="oc-legend" style={{ flexWrap: "wrap", gap: "6px 18px" }}>
            <span><b style={{ color: "#f3f4f6", fontWeight: 600 }}>{summary.multiKills}</b> multi-kills</span>
            <span><b style={{ color: "#f3f4f6", fontWeight: 600 }}>{unavailable}</b> without a clip</span>
          </div>
        </section>
      </div>

      <section>
        <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", margin: "0 2px 16px" }}>
          <h2 className="oc-h2">Clips from this match</h2>
          <span className="oc-sub" style={{ margin: 0 }}>{summary.clips.length} {summary.clips.length === 1 ? "moment" : "moments"}</span>
        </div>
        {summary.clips.length === 0 ? (
          <div className="oc-empty"><strong>No moments yet</strong><span>Kills, multi-kills and headshots from this match will appear here.</span></div>
        ) : (
          <div className="oc-clip-grid">
            {summary.clips.map((clip) => <ClipCard clip={clip} key={clip.id} onOpen={onOpenClip} />)}
          </div>
        )}
      </section>
    </div>
  );
}
