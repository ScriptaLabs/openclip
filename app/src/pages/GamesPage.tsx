import { ArrowUpRight, Crosshair, Sparkles } from "lucide-react";
import type { MatchRecord } from "../domain";
import { formatMode, getMatchStats } from "../domain";
import { MatchArtwork } from "../components/MatchArtwork";

interface GamesPageProps {
  matches: MatchRecord[];
  onOpenMatch: (id: string) => void;
  error: string | null;
}

function relativeLabel(startedAt: string): string {
  const date = new Date(startedAt);
  const elapsed = Math.max(0, Date.now() - date.getTime());
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 60) {
    const value = Math.max(1, minutes);
    return `${value} ${value === 1 ? "minute" : "minutes"} ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hours ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function GamesPage({ matches, onOpenMatch, error }: GamesPageProps) {
  const todayKey = new Date().toDateString();
  const today = matches.filter((match) => new Date(match.startedAt).toDateString() === todayKey);
  const earlier = matches.filter((match) => new Date(match.startedAt).toDateString() !== todayKey);

  const renderCard = (match: MatchRecord) => {
    const stats = getMatchStats(match);
    return (
      <button className="match-card" key={match.id} onClick={() => onOpenMatch(match.id)} type="button">
        <MatchArtwork map={match.map} tone={match.thumbnailTone} />
        <div className="match-card-body">
          <div className="match-card-heading">
            <div>
              <h3>{(match.map ?? "Unknown map").replace("de_", "")}</h3>
              <p>{formatMode(match.mode)} <span>•</span> {match.scoreTeamA}–{match.scoreTeamB}</p>
            </div>
            <ArrowUpRight size={18} />
          </div>
          <span className="time-label">{relativeLabel(match.startedAt)}</span>
          <div className="match-stats">
            <span><Crosshair size={14} /> {stats.kills} kills</span>
            <span>{stats.headshots} headshots</span>
            <span className="highlight-stat"><Sparkles size={14} /> {stats.highlights} highlights</span>
          </div>
        </div>
      </button>
    );
  };

  return (
    <div className="page games-page">
      <header className="page-header">
        <div>
          <h1>Games</h1>
          <p>Your recent Counter-Strike matches, organized around the moments that mattered.</p>
        </div>
        <div className="library-summary">
          <strong>{matches.length}</strong>
          <span>matches captured</span>
        </div>
      </header>

      {error && <div className="state-card error-state"><strong>Couldn’t load matches</strong><span>{error}</span></div>}
      {!error && matches.length === 0 && (
        <div className="state-card"><strong>No matches yet</strong><span>Start CS2 with GSI configured. OpenClip will add live matches here.</span></div>
      )}

      {today.length > 0 && (
        <section className="match-group">
          <h2>Today <span>{today.length}</span></h2>
          <div className="match-grid">{today.map(renderCard)}</div>
        </section>
      )}
      {earlier.length > 0 && (
        <section className="match-group">
          <h2>Earlier <span>{earlier.length}</span></h2>
          <div className="match-grid">{earlier.map(renderCard)}</div>
        </section>
      )}
    </div>
  );
}
