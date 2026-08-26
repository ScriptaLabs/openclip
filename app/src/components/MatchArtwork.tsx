interface MatchArtworkProps {
  map: string | null;
  tone: "mirage" | "inferno" | "ancient";
  compact?: boolean;
}

export function MatchArtwork({ map, tone, compact = false }: MatchArtworkProps) {
  const mapName = (map ?? "Unknown map").replace("de_", "");
  return (
    <div className={`match-artwork artwork-${tone} ${compact ? "compact" : ""}`}>
      <div className="map-geometry geometry-one" />
      <div className="map-geometry geometry-two" />
      <div className="artwork-vignette" />
      <span className="map-label">{mapName}</span>
    </div>
  );
}

