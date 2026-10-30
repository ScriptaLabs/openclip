import { useMemo } from "react";
import { ArrowUpRight } from "lucide-react";
import { matchWhen } from "../components/MatchSections";
import { dayLabel, type ClipEntry } from "../data/clips";
import { artworkForMap } from "../data/mapArtwork";
import { buildMatchSummary, type MatchSummary } from "../data/overview";
import { formatMode, originLabel, type MatchRecord } from "../domain";
import { mapLabel } from "../lib/maps";

interface MatchesPageProps {
  matches: MatchRecord[];
  clips: ClipEntry[];
  error: string | null;
  onOpenMatch: (id: string) => void;
}

function MatchCard({ summary, onOpen }: { summary: MatchSummary; onOpen: () => void }) {
  const { match } = summary;
  const art = artworkForMap(match.map);
  const maxKills = Math.max(1, ...summary.rounds.map((round) => round.kills));
  return (
    <button className="oc-match" onClick={onOpen} type="button">
      <div className="oc-match-art">
        {art && <img alt="" src={art} />}
        <div className="oc-match-title">
          <h3>{mapLabel(match.map)}</h3>
          {match.scoreTeamA !== null && match.scoreTeamB !== null && (
            <span>{match.scoreTeamA} – {match.scoreTeamB}</span>
          )}
        </div>
        <ArrowUpRight className="oc-match-arrow" size={18} />
      </div>
      <div className="oc-match-body">
        <span className="oc-match-when">{[originLabel(match.origin) ?? formatMode(match.mode), matchWhen(match)].join(" · ")}</span>
        {summary.rounds.length > 0 && (
          <div aria-hidden="true" className="oc-mini-bars" style={{ gridTemplateColumns: `repeat(${summary.rounds.length}, minmax(0, 1fr))` }}>
            {summary.rounds.map((round) => (
              <i
                className={round.saved ? "saved" : ""}
                key={round.number}
                style={{ height: `${Math.max(round.kills / maxKills, 0.08) * 100}%` }}
              />
            ))}
          </div>
        )}
        <div className="oc-match-stats">
          <span><b>{summary.kills}</b> kills</span>
          <span><b>{summary.headshotPercent}%</b> headshots</span>
          <span className="clips"><b>{summary.clipsSaved}</b> {summary.clipsSaved === 1 ? "clip" : "clips"}</span>
        </div>
      </div>
    </button>
  );
}

/** A workshop or community session that produced nothing to watch is noise, not a match. */
function isEmptyPractice(match: MatchRecord, clips: ClipEntry[]): boolean {
  if (!match.origin || match.origin === "official") return false;
  return match.moments.length === 0 && !clips.some((clip) => clip.matchId === match.id);
}

export function MatchesPage({ matches, clips, error, onOpenMatch }: MatchesPageProps) {
  const groups = useMemo(() => {
    const sorted = [...matches].sort((a, b) => new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime());
    const byDay = new Map<string, MatchSummary[]>();
    for (const match of sorted) {
      if (isEmptyPractice(match, clips)) continue;
      const label = dayLabel(new Date(match.startedAt).getTime());
      byDay.set(label, [...(byDay.get(label) ?? []), buildMatchSummary(match, clips)]);
    }
    return [...byDay.entries()];
  }, [clips, matches]);

  const shown = groups.reduce((total, [, summaries]) => total + summaries.length, 0);

  return (
    <div className="oc-page">
      <header className="oc-head">
        <div>
          <h1>Matches</h1>
          <p>Every game OpenClip followed, with the moments it saved.</p>
        </div>
        <span className="oc-count"><strong>{shown}</strong> {shown === 1 ? "match" : "matches"}</span>
      </header>

      {error && <div className="oc-error" role="alert">Couldn’t load matches: {error}</div>}
      {!error && shown === 0 && (
        <div className="oc-empty">
          <strong>No matches yet</strong>
          <span>Start Counter-Strike 2 with GSI configured. Matches appear here as you play.</span>
        </div>
      )}

      {groups.map(([label, summaries]) => (
        <section className="oc-group" key={label}>
          <h2>{label}</h2>
          <div className="oc-match-grid">
            {summaries.map((summary) => (
              <MatchCard key={summary.match.id} onOpen={() => onOpenMatch(summary.match.id)} summary={summary} />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
