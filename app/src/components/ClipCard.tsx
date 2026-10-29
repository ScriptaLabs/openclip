import { useEffect, useRef, useState } from "react";
import { Play, Star } from "lucide-react";
import { formatDuration, type ClipEntry } from "../data/clips";
import { ClipThumb } from "./ClipThumb";
import { loadClipMedia, type ClipMediaUrls } from "../data/media";
import { weaponIcon } from "../lib/weapons";

const empty: ClipMediaUrls = { clipUrl: null, thumbnailUrl: null };

/** Loads a clip's media once its card scrolls near the viewport. */
export function useClipMedia(clip: ClipEntry) {
  const ref = useRef<HTMLButtonElement>(null);
  const [media, setMedia] = useState<ClipMediaUrls>(empty);

  useEffect(() => {
    setMedia(empty);
    const node = ref.current;
    if (!clip.hasClip || !node) return;
    let active = true;
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        void loadClipMedia(clip.matchId, clip.id).then((urls) => { if (active) setMedia(urls); });
      },
      { rootMargin: "240px" },
    );
    observer.observe(node);
    return () => {
      active = false;
      observer.disconnect();
    };
  }, [clip.matchId, clip.id, clip.hasClip]);

  return { ref, media };
}

interface ClipCardProps {
  clip: ClipEntry;
  onOpen: (clip: ClipEntry) => void;
  /** Smaller title row, used where cards sit in a dense strip. */
  compact?: boolean;
}

export function ClipCard({ clip, onOpen, compact = false }: ClipCardProps) {
  const { ref, media } = useClipMedia(clip);
  const [previewing, setPreviewing] = useState(false);
  const delay = useRef<number>(undefined);
  const icon = weaponIcon(clip.weaponRaw);

  const startPreview = () => {
    if (!media.clipUrl) return;
    window.clearTimeout(delay.current);
    delay.current = window.setTimeout(() => setPreviewing(true), 260);
  };
  const stopPreview = () => {
    window.clearTimeout(delay.current);
    setPreviewing(false);
  };
  useEffect(() => () => window.clearTimeout(delay.current), []);

  const meta = [clip.mapName, clip.roundNumber ? `Round ${clip.roundNumber}` : null].filter(Boolean).join(" · ");

  return (
    <button
      className="oc-clip"
      disabled={!clip.hasClip}
      onBlur={stopPreview}
      onClick={() => onOpen(clip)}
      onFocus={startPreview}
      onMouseEnter={startPreview}
      onMouseLeave={stopPreview}
      ref={ref}
      type="button"
    >
      <div className="oc-thumb">
        <ClipThumb clip={clip} thumbnailUrl={media.thumbnailUrl} />
        {previewing && media.clipUrl && (
          <video
            autoPlay
            loop
            muted
            onLoadedMetadata={(event) => {
              const first = clip.kills[0]?.atMs ?? 0;
              event.currentTarget.currentTime = Math.max(0, first / 1000 - 1.5);
            }}
            playsInline
            src={media.clipUrl}
          />
        )}
        <span className={`oc-pill tl ${clip.big ? "big" : ""}`}>{clip.badge}</span>
        {clip.favorite && (
          <span className="oc-pill tr"><Star fill="currentColor" size={13} /></span>
        )}
        {clip.hasClip && !previewing && <span className="oc-play"><Play fill="currentColor" size={18} style={{ marginLeft: 2 }} /></span>}
        {previewing && <span className="oc-scrub"><i /></span>}
        <span className="oc-pill br">{clip.hasClip ? formatDuration(clip.durationMs) : "No clip"}</span>
      </div>
      <div className="oc-clip-body">
        <strong>{clip.title}</strong>
        <span className="oc-meta">
          {icon && !compact && <img alt={clip.weapon ?? ""} src={icon} />}
          <span>{clip.hasClip ? meta : "Clip unavailable"}</span>
        </span>
      </div>
    </button>
  );
}
