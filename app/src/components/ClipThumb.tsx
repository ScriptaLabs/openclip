import { useEffect, useState } from "react";
import { Film } from "lucide-react";
import type { ClipEntry } from "../data/clips";
import { artworkForMap } from "../data/mapArtwork";

/**
 * A clip's poster: the real frame when it loaded, then the map's artwork, then a neutral
 * placeholder. A broken image falls through to the next source instead of leaving a hole.
 */
export function ClipThumb({ clip, thumbnailUrl }: { clip: ClipEntry; thumbnailUrl: string | null }) {
  const art = artworkForMap(clip.match.map);
  const sources = [thumbnailUrl, art].filter((source): source is string => Boolean(source));
  const [skipped, setSkipped] = useState(0);
  useEffect(() => setSkipped(0), [thumbnailUrl, art]);
  const source = sources[skipped];
  return source
    ? <img alt="" onError={() => setSkipped((count) => count + 1)} src={source} />
    : <div className="oc-thumb-empty"><Film size={22} /></div>;
}
