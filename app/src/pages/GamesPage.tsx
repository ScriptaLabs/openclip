import { ArrowUpRight, Crosshair, Sparkles } from "lucide-react";
import type { MatchRecord } from "../domain";
import { formatMode, getMatchStats } from "../domain";
import { MatchArtwork } from "../components/MatchArtwork";

interface GamesPageProps {
  matches: MatchRecord[];
  onOpenMatch: (id: string) => void;
}

function relativeLabel(startedAt: string): string {
  const date = new Date(startedAt);
  return date.getUTCDate() === 26 ? (date.getUTCHours() >= 18 ? "42 minutes ago" : "Earlier today") : "Yesterday";
}

export function GamesPage({ matches, onOpenMatch }: GamesPageProps) {
  const today = matches.filter((match) => new Date(match.startedAt).getUTCDate() === 26);
  const earlier = matches.filter((match) => new Date(match.startedAt).getUTCDate() !== 26);

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
          <span className="eyebrow">Match library</span>
          <h1>Games</h1>
          <p>Your recent Counter-Strike matches, organized around the moments that mattered.</p>
        </div>
        <div className="library-summary">
          <strong>{matches.length}</strong>
          <span>matches captured</span>
        </div>
      </header>

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

