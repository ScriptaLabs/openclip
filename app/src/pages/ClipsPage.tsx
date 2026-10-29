import { useMemo, useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import { ClipCard } from "../components/ClipCard";
import { dayLabel, type ClipEntry } from "../data/clips";

type Filter = "all" | "multi" | "headshots" | "favorites";
type Sort = "newest" | "best";

interface ClipsPageProps {
  clips: ClipEntry[];
  error: string | null;
  onOpenClip: (clip: ClipEntry) => void;
}

const filters: Array<{ id: Filter; label: string; test: (clip: ClipEntry) => boolean }> = [
  { id: "all", label: "All", test: () => true },
  { id: "multi", label: "Multi-kills", test: (clip) => clip.kills.length >= 2 },
  { id: "headshots", label: "Headshots", test: (clip) => clip.headshots > 0 },
  { id: "favorites", label: "Favorites", test: (clip) => clip.favorite },
];

export function ClipsPage({ clips, error, onOpenClip }: ClipsPageProps) {
  const [filter, setFilter] = useState<Filter>("all");
  const [map, setMap] = useState("all");
  const [sort, setSort] = useState<Sort>("newest");
  const [query, setQuery] = useState("");

  const maps = useMemo(() => [...new Set(clips.map((clip) => clip.mapName))].sort(), [clips]);
  const needle = query.trim().toLowerCase();

  const visible = useMemo(() => {
    const test = filters.find((item) => item.id === filter)!.test;
    const matching = clips.filter((clip) =>
      test(clip)
      && (map === "all" || clip.mapName === map)
      && (!needle || `${clip.title} ${clip.weapon ?? ""} ${clip.mapName}`.toLowerCase().includes(needle)));
    return sort === "best"
      ? [...matching].sort((a, b) => b.moment.importance - a.moment.importance)
      : matching;
  }, [clips, filter, map, needle, sort]);

  const groups = useMemo(() => {
    if (sort === "best") return [{ label: "Highest scoring", clips: visible }];
    const byDay = new Map<string, ClipEntry[]>();
    for (const clip of visible) {
      const label = dayLabel(clip.at);
      byDay.set(label, [...(byDay.get(label) ?? []), clip]);
    }
    return [...byDay.entries()].map(([label, entries]) => ({ label, clips: entries }));
  }, [sort, visible]);

  return (
    <div className="oc-page">
      <header className="oc-head">
        <div>
          <h1>Clips</h1>
          <p>Saved automatically as you play.</p>
        </div>
        <div className="oc-actions">
          <label className="oc-field search">
            <Search size={16} />
            <input aria-label="Search clips" onChange={(event) => setQuery(event.target.value)} placeholder="Search clips" type="text" value={query} />
          </label>
          <label className="oc-field">
            <select aria-label="Sort clips" onChange={(event) => setSort(event.target.value as Sort)} value={sort}>
              <option value="newest">Newest</option>
              <option value="best">Best first</option>
            </select>
            <ChevronDown size={14} />
          </label>
        </div>
      </header>

      <div className="oc-chips">
        {filters.map((item) => (
          <button
            aria-pressed={filter === item.id}
            className={`oc-chip ${filter === item.id ? "active" : ""}`}
            key={item.id}
            onClick={() => setFilter(item.id)}
            type="button"
          >
            {item.label}<small>{clips.filter(item.test).length}</small>
          </button>
        ))}
        <span className="oc-chip-sep" />
        <label className="oc-field" style={{ height: 36, borderRadius: 99, padding: "0 12px 0 14px", background: "transparent" }}>
          <select aria-label="Filter by map" onChange={(event) => setMap(event.target.value)} value={map}>
            <option value="all">All maps</option>
            {maps.map((name) => <option key={name} value={name}>{name}</option>)}
          </select>
          <ChevronDown size={13} />
        </label>
      </div>

      {error && <div className="oc-error" role="alert">Couldn’t load clips: {error}</div>}
      {!error && clips.length === 0 && (
        <div className="oc-empty">
          <strong>No clips yet</strong>
          <span>Play a match with OpenClip running. Kills, multi-kills and headshots appear here automatically.</span>
        </div>
      )}
      {clips.length > 0 && visible.length === 0 && (
        <div className="oc-empty"><strong>Nothing matches</strong><span>Try a different filter or search.</span></div>
      )}

      {groups.map((group) => (
        <section className="oc-group" key={group.label}>
          <h2>{group.label}</h2>
          <div className="oc-clip-grid">
            {group.clips.map((clip) => <ClipCard clip={clip} key={clip.id} onOpen={onOpenClip} />)}
          </div>
        </section>
      ))}
    </div>
  );
}
