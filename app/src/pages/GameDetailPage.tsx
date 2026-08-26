import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Check, ChevronRight, Download, Headphones, Play, Sparkles, Star } from "lucide-react";
import type { MatchRecord, Moment } from "../domain";
import { formatMode } from "../domain";
import { exportInterestingMoments, openMomentClip } from "../data/matchRepository";

interface GameDetailPageProps {
  match: MatchRecord;
  onBack: () => void;
}

function momentSummary(moment: Moment): string {
  const kills = moment.events.filter((event) => event.eventType === "kill").length;
  const headshots = moment.events.filter((event) => event.eventType === "headshot").length;
  return `${kills} ${kills === 1 ? "kill" : "kills"} • ${headshots} ${headshots === 1 ? "headshot" : "headshots"}`;
}

function durationLabel(match: MatchRecord): string {
  if (!match.endedAt) return "In progress";
  const minutes = Math.max(1, Math.round((new Date(match.endedAt).getTime() - new Date(match.startedAt).getTime()) / 60_000));
  return `${minutes} minutes`;
}

export function GameDetailPage({ match, onBack }: GameDetailPageProps) {
  const [exportOpen, setExportOpen] = useState(false);
  const [destination, setDestination] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const mapName = (match.map ?? "Unknown map").replace("de_", "");
  const roundById = new Map(match.rounds.map((round) => [round.id, round]));
  const capturedMoments = match.moments.filter((moment) => moment.clipPath && (moment.importance >= 10 || moment.isFavorite));

  const chooseExportFolder = async () => {
    const selected = await open({ directory: true, multiple: false, title: "Export interesting moments" });
    if (selected) setDestination(selected);
  };

  const exportMoments = async () => {
    if (!destination) return;
    setExporting(true);
    setMessage(null);
    try {
      const paths = await exportInterestingMoments(match.id, destination);
      setMessage(`${paths.length} ${paths.length === 1 ? "moment" : "moments"} exported`);
      setExportOpen(false);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="page detail-page">
      <button className="back-button" onClick={onBack} type="button">
        <ArrowLeft size={16} /> Games
      </button>

      <header className="detail-hero">
        <div>
          <h1>{mapName}</h1>
          <div className="match-result-line">
            <span>{formatMode(match.mode)}</span>
            <strong>{match.scoreTeamA} <i>—</i> {match.scoreTeamB}</strong>
            <span>Today • {durationLabel(match)}</span>
          </div>
        </div>
        <button className="primary-button" disabled={capturedMoments.length === 0} onClick={() => setExportOpen(true)} type="button">
          <Download size={17} /> Export interesting moments
        </button>
      </header>

      <section className="timeline-panel">
        <div className="section-heading">
          <div>
            <h2>Timeline</h2>
          </div>
          <div className="timeline-legend"><i /> Captured moment</div>
        </div>
        <div className="timeline-scroll">
          <div className="timeline-line" />
          {match.rounds.map((round) => {
            const moments = match.moments.filter((moment) => moment.roundId === round.id);
            return (
              <div className={`round-node ${moments.length > 0 ? "has-moment" : ""}`} key={round.id}>
                <span className="round-marker">{moments.length > 0 && <i />}</span>
                <strong>R{round.number}</strong>
              </div>
            );
          })}
        </div>
      </section>

      <section className="detail-section">
        <div className="section-heading">
          <div>
            <h2>Highlights</h2>
          </div>
          <span className="section-count">{match.moments.length} moments</span>
        </div>
        <div className="highlight-list">
          {match.moments.map((moment) => {
            const round = moment.roundId ? roundById.get(moment.roundId) : undefined;
            const seconds = Math.round((moment.endOffsetMs - moment.startOffsetMs) / 1000);
            return (
              <button className="highlight-row" disabled={!moment.clipPath} key={moment.id} onClick={() => void openMomentClip(match.id, moment.id).catch((error) => setMessage(String(error)))} type="button">
                <div className={`moment-thumbnail tone-${moment.thumbnailTone}`}>
                  <div className="thumbnail-grid" />
                  <span className="play-button"><Play size={16} fill="currentColor" /></span>
                  <small>0:{String(seconds).padStart(2, "0")}</small>
                </div>
                <div className="moment-copy">
                  <span className="moment-round">Round {round?.number ?? "—"}</span>
                  <h3>{moment.title}</h3>
                  <p>{moment.weapon} <span>•</span> {momentSummary(moment)}</p>
                </div>
                <div className="moment-meta">
                  {moment.isFavorite && <Star size={15} fill="currentColor" />}
                  <span>{Math.round(moment.importance)} pts</span>
                </div>
                {moment.clipPath ? <ChevronRight size={18} className="row-chevron" /> : <small className="clip-unavailable">Clip unavailable</small>}
              </button>
            );
          })}
        </div>
        {match.moments.length === 0 && <div className="inline-empty">No highlight windows have been created for this match.</div>}
      </section>

      <section className="detail-section rounds-section">
        <div className="section-heading">
          <div>
            <h2>All rounds</h2>
          </div>
        </div>
        <div className="round-list">
          {match.rounds.map((round) => {
            const kills = round.events.filter((event) => event.eventType === "kill").length;
            const captured = match.moments.some((moment) => moment.roundId === round.id);
            return (
              <div className="round-row" key={round.id}>
                <span className={`round-outcome ${round.number <= (match.scoreTeamA ?? 0) ? "won" : "lost"}`}>
                  {round.number <= (match.scoreTeamA ?? 0) ? <Check size={13} /> : "—"}
                </span>
                <strong>Round {round.number}</strong>
                <span>{kills > 0 ? `${kills} ${kills === 1 ? "kill" : "kills"}` : "No detected kills"}</span>
                {captured ? <em><Sparkles size={13} /> Highlight saved</em> : <small>No captured moments</small>}
              </div>
            );
          })}
        </div>
      </section>

      <footer className="capture-note">
        <Headphones size={15} /> Events and captured video remain local to this computer.
      </footer>
      {message && <div className="inline-message" role="status">{message}</div>}
      {exportOpen && <div className="dialog-backdrop" role="presentation"><section aria-labelledby="export-title" aria-modal="true" className="export-dialog" role="dialog"><h2 id="export-title">Export interesting moments</h2><p>{capturedMoments.length} captured {capturedMoments.length === 1 ? "moment" : "moments"} will be copied as separate MP4 files.</p><label><span>Destination</span><button className="path-button" onClick={() => void chooseExportFolder()} type="button"><Download size={16} /><span>{destination ?? "Choose folder"}</span></button></label><div className="dialog-actions"><button className="secondary-button" onClick={() => setExportOpen(false)} type="button">Cancel</button><button className="primary-button" disabled={!destination || exporting} onClick={() => void exportMoments()} type="button">{exporting ? "Exporting…" : "Export"}</button></div></section></div>}
    </div>
  );
}
