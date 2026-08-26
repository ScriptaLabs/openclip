interface MatchArtworkProps {
  map: string | null;
  tone: "mirage" | "inferno" | "ancient";
  compact?: boolean;
}

import { artworkForMap } from "../data/mapArtwork";

export function MatchArtwork({ map, tone, compact = false }: MatchArtworkProps) {
  const mapName = (map ?? "Unknown map").replace("de_", "");
  const artwork = artworkForMap(map);
  return (
    <div className={`match-artwork artwork-${tone} ${compact ? "compact" : ""}`}>
      {artwork && <img alt="" aria-hidden="true" src={artwork} />}
      <div className="artwork-vignette" />
      <span className="map-label">{mapName}</span>
    </div>
  );
}
