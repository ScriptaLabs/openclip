import { Play } from "lucide-react";
import { useClipMedia } from "./ClipCard";
import { ClipThumb } from "./ClipThumb";
import { formatDuration, type ClipEntry } from "../data/clips";
import { artworkForMap } from "../data/mapArtwork";
import type { MatchSummary } from "../data/overview";
import { formatMode, type MatchRecord } from "../domain";
import { mapLabel, serverLabel } from "../lib/maps";
import { weaponIcon } from "../lib/weapons";

export function matchWhen(match: MatchRecord): string {
  const started = new Date(match.startedAt);
  const day = started.toDateString() === new Date().toDateString()
    ? "Today"
    : started.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const time = started.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (!match.endedAt) return `${day}, ${time} · In progress`;
  const minutes = Math.max(1, Math.round((new Date(match.endedAt).getTime() - started.getTime()) / 60_000));
  return `${day}, ${time} · ${minutes} min`;
}

export function MatchRecap({ summary }: { summary: MatchSummary }) {
  const { match } = summary;
  const art = artworkForMap(match.map);
  const server = serverLabel(match.map);
  return (
    <section className="oc-card oc-recap">
      <div className="oc-recap-hero">
        {art && <img alt="" src={art} />}
        <div className="oc-recap-title">
          <span>{formatMode(match.mode)} · {matchWhen(match)}</span>
          <div>
            <h2>{mapLabel(match.map)}</h2>
            {match.scoreTeamA !== null && match.scoreTeamB !== null && (
              <span className="oc-score">{match.scoreTeamA} <em>–</em> {match.scoreTeamB}</span>
            )}
          </div>
          {server && <p className="oc-server">{server}</p>}
        </div>
      </div>
      <div className="oc-stats">
        <div><strong>{summary.kills}</strong><span>Kills</span></div>
        <div><strong>{summary.headshotPercent}%</strong><span>Headshots</span></div>
        <div><strong>{summary.multiKills}</strong><span>Multi-kills</span></div>
        <div><strong>{summary.rounds.length}</strong><span>Rounds</span></div>
        <div><strong>{summary.clipsSaved}</strong><span>Clips saved</span></div>
      </div>
    </section>
  );
}

export function BestMoment({ clip, onOpen }: { clip: ClipEntry; onOpen: (clip: ClipEntry) => void }) {
  const { ref, media } = useClipMedia(clip);
  const icon = weaponIcon(clip.weaponRaw);
  return (
    <section className="oc-card oc-best">
      <div className="oc-best-head"><h2 className="oc-h2">Best moment</h2><span>Score {Math.round(clip.moment.importance)}</span></div>
      <button className="oc-clip" onClick={() => onOpen(clip)} ref={ref} type="button">
        <div className="oc-thumb">
          <ClipThumb clip={clip} thumbnailUrl={media.thumbnailUrl} />
          <span className={`oc-pill tl ${clip.big ? "big" : ""}`}>{clip.badge}</span>
          <span className="oc-play" style={{ opacity: 1, transform: "none" }}><Play fill="currentColor" size={18} style={{ marginLeft: 2 }} /></span>
          <span className="oc-pill br">{formatDuration(clip.durationMs)}</span>
        </div>
      </button>
      <div className="oc-best-foot">
        <div>
          <strong>{clip.title}</strong>
          <span className="oc-meta">
            {icon && <img alt={clip.weapon ?? ""} src={icon} />}
            <span>{clip.roundNumber ? `Round ${clip.roundNumber}` : clip.mapName}{clip.headshots > 0 ? ` · ${clip.headshots} ${clip.headshots === 1 ? "headshot" : "headshots"}` : ""}</span>
          </span>
        </div>
        <button className="oc-btn small" onClick={() => onOpen(clip)} style={{ background: "#f3f4f6", color: "#0a0b0e", borderColor: "transparent" }} type="button">Watch</button>
      </div>
    </section>
  );
}

export function RoundsChart({ summary, onOpenClip }: { summary: MatchSummary; onOpenClip: (clip: ClipEntry) => void }) {
  const maxKills = Math.max(1, ...summary.rounds.map((round) => round.kills));
  if (summary.rounds.length === 0) return null;
  return (
    <section className="oc-card oc-rounds-card">
      <div className="oc-rounds-head">
        <div>
          <h2 className="oc-h2">Your kills by round</h2>
          <p className="oc-sub">{summary.kills} {summary.kills === 1 ? "kill" : "kills"} over {summary.rounds.length} rounds. Highlighted rounds have a saved clip.</p>
        </div>
        <div className="oc-legend"><span><i /> Clip saved</span><span><i className="dim" /> No clip</span></div>
      </div>
      <div className="oc-bars" style={{ gridTemplateColumns: `repeat(${summary.rounds.length}, minmax(0, 1fr))` }}>
        {summary.rounds.map((round) => (
          <button
            aria-label={`Round ${round.number}: ${round.kills} ${round.kills === 1 ? "kill" : "kills"}${round.saved ? ", clip saved" : ""}`}
            className={`oc-bar ${round.kills === 0 ? "empty" : ""} ${round.saved ? "saved" : ""}`}
            disabled={!round.clip}
            key={round.number}
            onClick={() => round.clip && onOpenClip(round.clip)}
            type="button"
          >
            <div><i style={{ height: `${(round.kills / maxKills) * 100}%` }} /></div>
            <span>{round.number}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

export function WeaponsCard({ summary }: { summary: MatchSummary }) {
  const maxWeapon = Math.max(1, ...summary.weapons.map((weapon) => weapon.kills));
  if (summary.weapons.length === 0) return null;
  return (
    <section className="oc-card">
      <h2 className="oc-h2" style={{ marginBottom: 16 }}>Kills by weapon</h2>
      {summary.weapons.map((weapon) => {
        const icon = weaponIcon(weapon.raw);
        return (
          <div className="oc-weapon" key={weapon.raw}>
            <div className="icon">{icon && <img alt="" src={icon} />}</div>
            <strong>{weapon.label}</strong>
            <div className="oc-meter"><i style={{ width: `${(weapon.kills / maxWeapon) * 100}%` }} /></div>
            <span>{weapon.kills}</span>
          </div>
        );
      })}
    </section>
  );
}
