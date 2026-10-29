import { ChevronRight } from "lucide-react";
import { ClipCard } from "../components/ClipCard";
import { BestMoment, MatchRecap, RoundsChart, WeaponsCard } from "../components/MatchSections";
import type { ClipEntry } from "../data/clips";
import type { Overview } from "../data/overview";

interface OverviewPageProps {
  overview: Overview;
  error: string | null;
  onOpenClip: (clip: ClipEntry) => void;
  onOpenMatch: (id: string) => void;
  onSeeAll: () => void;
}

export function OverviewPage({ overview, error, onOpenClip, onOpenMatch, onSeeAll }: OverviewPageProps) {
  const { summary } = overview;
  const { captured, missed } = overview.health;

  return (
    <div className="oc-page">
      <header className="oc-head">
        <div>
          <h1>Overview</h1>
          <p>Your last match, and what OpenClip captured from it.</p>
        </div>
        {summary && (
          <button className="oc-btn" onClick={() => onOpenMatch(summary.match.id)} type="button">
            Open match <ChevronRight size={15} />
          </button>
        )}
      </header>

      {error && <div className="oc-error" role="alert">Couldn’t load your data: {error}</div>}

      {!summary && !error && (
        <div className="oc-empty">
          <strong>No matches yet</strong>
          <span>Start Counter-Strike 2 and OpenClip will build your recap here.</span>
        </div>
      )}

      {summary && (
        <>
          <div className="oc-row-layout">
            <MatchRecap summary={summary} />
            {summary.best && <BestMoment clip={summary.best} onOpen={onOpenClip} />}
          </div>
          <RoundsChart onOpenClip={onOpenClip} summary={summary} />
          <div className="oc-two">
            <WeaponsCard summary={summary} />
            <section className="oc-card">
              <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 16 }}>
                <h2 className="oc-h2">Capture health</h2>
                <span className={`oc-health-badge ${missed > 0 ? "warn" : ""}`}><i />{missed > 0 ? "Some missed" : "All good"}</span>
              </div>
              <div className="oc-big-number">{captured} <span>of {captured + missed}</span></div>
              <p className="oc-sub" style={{ margin: "6px 0 18px", fontSize: 13.5 }}>moments were saved as clips.</p>
              <div className="oc-split">
                <i className="ok" style={{ flex: Math.max(captured, 0.001) }} />
                {missed > 0 && <i className="miss" style={{ flex: missed }} />}
              </div>
              <div className="oc-legend" style={{ flexWrap: "wrap", gap: "6px 18px" }}>
                <span><b style={{ color: "#f3f4f6", fontWeight: 600 }}>{captured}</b> captured</span>
                <span><b style={{ color: "#f3f4f6", fontWeight: 600 }}>{missed}</b> without a clip</span>
              </div>
            </section>
          </div>
        </>
      )}

      {overview.recent.length > 0 && (
        <section>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", margin: "0 2px 16px" }}>
            <h2 className="oc-h2">Recent clips</h2>
            <button className="oc-nav-item" onClick={onSeeAll} style={{ width: "auto", padding: "4px 8px" }} type="button">See all <ChevronRight size={14} /></button>
          </div>
          <div className="oc-recent-grid">
            {overview.recent.map((clip) => <ClipCard clip={clip} compact key={clip.id} onOpen={onOpenClip} />)}
          </div>
        </section>
      )}
    </div>
  );
}
