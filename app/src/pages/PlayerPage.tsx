import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import {
  ChevronLeft, FolderOpen, Maximize, Pause, Play, SkipBack, SkipForward, Star, Volume2, VolumeX,
} from "lucide-react";
import { formatDuration, timeLabel, type ClipEntry } from "../data/clips";
import { artworkForMap } from "../data/mapArtwork";
import { ClipThumb } from "../components/ClipThumb";
import { useClipMedia } from "../components/ClipCard";
import { formatMode } from "../domain";
import { loadClipMedia, revealMomentClip, setMomentFavorite, type ClipMediaUrls } from "../data/media";
import { headshotIcon, weaponIcon } from "../lib/weapons";

interface PlayerPageProps {
  clip: ClipEntry;
  clips: ClipEntry[];
  onBack: () => void;
  onSelect: (clip: ClipEntry) => void;
  onOpenMatch: (matchId: string) => void;
  onChanged: () => void;
}

const SPEEDS = [0.5, 1, 1.5, 2];
const IDLE_MS = 2_400;

function storedVolume(): number {
  try {
    const value = Number(window.localStorage.getItem("openclip.playerVolume"));
    return Number.isFinite(value) && value > 0 ? Math.min(1, value) : 0.8;
  } catch {
    return 0.8;
  }
}

function RelatedClip({ clip, onSelect }: { clip: ClipEntry; onSelect: (clip: ClipEntry) => void }) {
  const { ref, media } = useClipMedia(clip);
  return (
    <button className="oc-related" onClick={() => onSelect(clip)} ref={ref} type="button">
      <div className="oc-thumb">
        <ClipThumb clip={clip} thumbnailUrl={media.thumbnailUrl} />
        <span className="oc-pill br" style={{ right: 5, bottom: 5, height: 18, padding: "0 6px", fontSize: 11 }}>{formatDuration(clip.durationMs)}</span>
      </div>
      <div>
        <strong>{clip.title}</strong>
        <span>{clip.roundNumber ? `Round ${clip.roundNumber}` : clip.mapName}</span>
      </div>
    </button>
  );
}

export function PlayerPage({ clip, clips, onBack, onSelect, onOpenMatch, onChanged }: PlayerPageProps) {
  const frame = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const timeline = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const idleTimer = useRef<number>(undefined);

  const [media, setMedia] = useState<ClipMediaUrls | null>(null);
  const [failed, setFailed] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(storedVolume);
  const [muted, setMuted] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [awake, setAwake] = useState(true);
  const [hoverKill, setHoverKill] = useState<number | null>(null);
  const [favorite, setFavorite] = useState(clip.favorite);
  const [notice, setNotice] = useState<string | null>(null);

  const playable = useMemo(() => clips.filter((entry) => entry.hasClip), [clips]);
  const index = playable.findIndex((entry) => entry.id === clip.id);
  const newer = index > 0 ? playable[index - 1] : undefined;
  const older = index >= 0 ? playable[index + 1] : undefined;
  const related = useMemo(
    () => playable.filter((entry) => entry.matchId === clip.matchId && entry.id !== clip.id).slice(0, 3),
    [clip.id, clip.matchId, playable],
  );

  useEffect(() => {
    setMedia(null);
    setFailed(false);
    setPlaying(false);
    setTime(0);
    setDuration(0);
    setNotice(null);
    let active = true;
    void loadClipMedia(clip.matchId, clip.id).then((urls) => {
      if (!active) return;
      setMedia(urls);
      if (!urls.clipUrl) setFailed(true);
    });
    return () => { active = false; };
  }, [clip.id, clip.matchId]);

  // Kept separate so toggling a favorite never restarts playback.
  useEffect(() => setFavorite(clip.favorite), [clip.favorite]);

  useEffect(() => {
    const element = video.current;
    if (!element) return;
    element.volume = volume;
    element.muted = muted;
    element.playbackRate = speed;
  }, [volume, muted, speed, media]);

  const wake = useCallback(() => {
    setAwake(true);
    window.clearTimeout(idleTimer.current);
    idleTimer.current = window.setTimeout(() => setAwake(false), IDLE_MS);
  }, []);
  useEffect(() => {
    wake();
    return () => window.clearTimeout(idleTimer.current);
  }, [wake, clip.id]);

  const toggle = useCallback(() => {
    const element = video.current;
    if (!element) return;
    if (element.paused) void element.play().catch(() => {});
    else element.pause();
  }, []);

  const seekTo = useCallback((seconds: number) => {
    const element = video.current;
    if (!element || !Number.isFinite(element.duration)) return;
    element.currentTime = Math.min(Math.max(0, seconds), element.duration);
    setTime(element.currentTime);
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName)) return;
      if ((event.key === " " || event.key === "k") && target?.tagName !== "BUTTON") {
        event.preventDefault();
        toggle();
      } else if (event.key === "ArrowLeft") {
        seekTo((video.current?.currentTime ?? 0) - 5);
      } else if (event.key === "ArrowRight") {
        seekTo((video.current?.currentTime ?? 0) + 5);
      } else if (event.key === "f") {
        void frame.current?.requestFullscreen?.().catch(() => {});
      } else if (event.key === "m") {
        setMuted((value) => !value);
      } else if (event.key === "Escape" && !document.fullscreenElement) {
        onBack();
      } else {
        return;
      }
      wake();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onBack, seekTo, toggle, wake]);

  const seekFromPointer = (event: PointerEvent) => {
    const rect = timeline.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return;
    seekTo(Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)) * (video.current?.duration ?? 0));
  };

  const toggleFavorite = async () => {
    const next = !favorite;
    setFavorite(next);
    try {
      await setMomentFavorite(clip.matchId, clip.id, next);
      onChanged();
    } catch (error) {
      setFavorite(!next);
      setNotice(error instanceof Error ? error.message : String(error));
    }
  };

  const reveal = () => {
    revealMomentClip(clip.matchId, clip.id).catch((error) => setNotice(String(error)));
  };

  const progress = duration > 0 ? time / duration : 0;
  const killPosition = (atMs: number) => (duration > 0 ? Math.min(1, atMs / 1000 / duration) : 0);
  const activeKill = clip.kills.reduce((found, kill, position) => (kill.atMs / 1000 <= time + 0.4 ? position : found), -1);
  const art = artworkForMap(clip.match.map);
  const weaponImage = weaponIcon(clip.weaponRaw);
  const showControls = awake || !playing;
  const hovered = hoverKill === null ? null : clip.kills[hoverKill];

  return (
    <div className="oc-page">
      <div className="oc-player-top">
        <button className="oc-btn" onClick={onBack} type="button"><ChevronLeft size={16} /> Back</button>
        <div className="oc-actions">
          <button aria-label={favorite ? "Remove from favorites" : "Add to favorites"} aria-pressed={favorite} className={`oc-icon-btn ${favorite ? "on" : ""}`} onClick={() => void toggleFavorite()} type="button">
            <Star fill={favorite ? "currentColor" : "none"} size={16} />
          </button>
          <button className="oc-btn" onClick={reveal} type="button"><FolderOpen size={16} /> Show in folder</button>
        </div>
      </div>

      <div className="oc-player-layout">
        <div className="oc-player-main">
          <div
            className={`oc-video ${showControls ? "show-controls" : ""}`}
            onMouseLeave={() => playing && setAwake(false)}
            onMouseMove={wake}
            ref={frame}
          >
            {media?.clipUrl && (
              <video
                autoPlay
                onClick={toggle}
                onDoubleClick={() => void frame.current?.requestFullscreen?.().catch(() => {})}
                onEnded={() => setPlaying(false)}
                onError={() => setFailed(true)}
                onLoadedMetadata={(event) => setDuration(event.currentTarget.duration)}
                onPause={() => setPlaying(false)}
                onPlay={() => setPlaying(true)}
                onTimeUpdate={(event) => { if (!dragging.current) setTime(event.currentTarget.currentTime); }}
                playsInline
                ref={video}
                src={media.clipUrl}
              />
            )}
            {!media && <div className="oc-video-state">Loading clip…</div>}
            {failed && <div className="oc-video-state">This clip couldn’t be played. The file may have been moved.</div>}

            <div className="oc-controls">
              <div
                className="oc-timeline"
                onPointerDown={(event) => { dragging.current = true; event.currentTarget.setPointerCapture(event.pointerId); seekFromPointer(event); }}
                onPointerMove={(event) => { if (dragging.current) seekFromPointer(event); }}
                onPointerUp={(event) => { dragging.current = false; event.currentTarget.releasePointerCapture(event.pointerId); }}
                ref={timeline}
              >
                <div className="oc-track">
                  <div className="fill" style={{ width: `${progress * 100}%` }} />
                  {clip.kills.map((kill, position) => (
                    <button
                      aria-label={`Jump to kill ${kill.n}`}
                      className={`oc-marker ${kill.atMs / 1000 <= time + 0.4 ? "passed" : ""}`}
                      key={kill.n}
                      onBlur={() => setHoverKill(null)}
                      onClick={(event) => { event.stopPropagation(); seekTo(kill.atMs / 1000 - 1.5); }}
                      onFocus={() => setHoverKill(position)}
                      onMouseEnter={() => setHoverKill(position)}
                      onMouseLeave={() => setHoverKill(null)}
                      onPointerDown={(event) => event.stopPropagation()}
                      style={{ left: `${killPosition(kill.atMs) * 100}%` }}
                      type="button"
                    />
                  ))}
                  <span className="thumb" style={{ left: `${progress * 100}%` }} />
                  {hovered && (
                    <div className="oc-tip" style={{ left: `${killPosition(hovered.atMs) * 100}%` }}>
                      <b>Kill {hovered.n}</b>
                      {weaponIcon(hovered.weaponRaw) && <img alt={hovered.weapon ?? ""} src={weaponIcon(hovered.weaponRaw)} />}
                      {hovered.headshot && headshotIcon && <img alt="Headshot" src={headshotIcon} />}
                    </div>
                  )}
                </div>
              </div>
              <div className="oc-ctrl-row">
                <button aria-label="Newer clip" className="oc-ctrl" disabled={!newer} onClick={() => newer && onSelect(newer)} type="button"><SkipBack fill="currentColor" size={17} /></button>
                <button aria-label={playing ? "Pause" : "Play"} className="oc-ctrl main" onClick={toggle} type="button">
                  {playing ? <Pause fill="currentColor" size={18} /> : <Play fill="currentColor" size={18} style={{ marginLeft: 2 }} />}
                </button>
                <button aria-label="Older clip" className="oc-ctrl" disabled={!older} onClick={() => older && onSelect(older)} type="button"><SkipForward fill="currentColor" size={17} /></button>
                <span className="oc-time">{formatDuration(time * 1000)} <span>/ {formatDuration(duration * 1000)}</span></span>
                <div className="oc-grow" />
                <button aria-label={muted ? "Unmute" : "Mute"} className="oc-ctrl" onClick={() => setMuted((value) => !value)} type="button">
                  {muted || volume === 0 ? <VolumeX size={18} /> : <Volume2 size={18} />}
                </button>
                <input
                  aria-label="Volume"
                  className="oc-vol"
                  max={1}
                  min={0}
                  onChange={(event) => {
                    const next = Number(event.target.value);
                    setVolume(next);
                    setMuted(next === 0);
                    try { window.localStorage.setItem("openclip.playerVolume", String(next)); } catch { /* optional */ }
                  }}
                  step={0.05}
                  type="range"
                  value={muted ? 0 : volume}
                />
                <button aria-label="Playback speed" className="oc-speed" onClick={() => setSpeed((value) => SPEEDS[(SPEEDS.indexOf(value) + 1) % SPEEDS.length]!)} type="button">{speed}×</button>
                <button aria-label="Fullscreen" className="oc-ctrl" onClick={() => void frame.current?.requestFullscreen?.().catch(() => {})} type="button"><Maximize size={17} /></button>
              </div>
            </div>
          </div>

          <div className="oc-player-title">
            <div>
              <h1>{clip.title}<span className="oc-tag">{clip.badge}</span></h1>
              <p className="oc-meta">
                {weaponImage && <img alt={clip.weapon ?? ""} src={weaponImage} />}
                <span>
                  {[clip.mapName, clip.roundNumber ? `Round ${clip.roundNumber}` : null,
                    clip.headshots > 0 ? `${clip.headshots} ${clip.headshots === 1 ? "headshot" : "headshots"}` : null,
                    timeLabel(clip.at)].filter(Boolean).join(" · ")}
                </span>
              </p>
            </div>
            <button className="oc-match-link" onClick={() => onOpenMatch(clip.matchId)} type="button">
              {art && <img alt="" src={art} />}
              <div>
                <strong>{formatMode(clip.match.mode)}{clip.match.scoreTeamA !== null ? ` · ${clip.match.scoreTeamA}–${clip.match.scoreTeamB}` : ""}</strong>
                <span>View match</span>
              </div>
            </button>
          </div>
          {notice && <div className="oc-error" role="alert" style={{ marginTop: 18 }}>{notice}</div>}
        </div>

        <aside className="oc-side">
          {clip.kills.length > 0 && (
            <section className="oc-card oc-panel">
              <h2 className="oc-h2">Kills in this clip</h2>
              {clip.kills.map((kill, position) => {
                const icon = weaponIcon(kill.weaponRaw);
                return (
                  <button
                    className={`oc-kill ${position === activeKill ? "active" : ""}`}
                    key={kill.n}
                    onClick={() => seekTo(kill.atMs / 1000 - 1.5)}
                    type="button"
                  >
                    <span className="n">{kill.n}</span>
                    <span className="w">{icon && <img alt="" src={icon} />}{kill.weapon ?? "Kill"}</span>
                    {kill.headshot && <span className="mod">{headshotIcon && <img alt="" src={headshotIcon} />}Headshot</span>}
                    <span className="t">{formatDuration(kill.atMs)}</span>
                  </button>
                );
              })}
            </section>
          )}
          {related.length > 0 && (
            <section>
              <h2 className="oc-h2" style={{ margin: "0 4px 12px" }}>More from this match</h2>
              <div style={{ display: "grid", gap: 10 }}>
                {related.map((entry) => <RelatedClip clip={entry} key={entry.id} onSelect={onSelect} />)}
              </div>
            </section>
          )}
        </aside>
      </div>
    </div>
  );
}
