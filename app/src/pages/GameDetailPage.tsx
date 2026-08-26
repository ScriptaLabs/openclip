import { ArrowLeft, Check, ChevronRight, Download, Headphones, Play, Sparkles, Star } from "lucide-react";
import type { MatchRecord, Moment } from "../domain";
import { formatMode } from "../domain";

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
  const mapName = (match.map ?? "Unknown map").replace("de_", "");
  const roundById = new Map(match.rounds.map((round) => [round.id, round]));

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
        <button className="primary-button" type="button">
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
              <button className="highlight-row" key={moment.id} type="button">
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
                <ChevronRight size={18} className="row-chevron" />
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
        <Headphones size={15} /> Game events are local metadata. Video capture is not active yet.
      </footer>
    </div>
  );
}
