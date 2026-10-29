import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";

export interface ClipMediaUrls {
  clipUrl: string | null;
  thumbnailUrl: string | null;
}

const none: ClipMediaUrls = { clipUrl: null, thumbnailUrl: null };
const cache = new Map<string, Promise<ClipMediaUrls>>();

// Each request may start FFmpeg to make a thumbnail, so only a few run at once.
const MAX_CONCURRENT = 3;
let running = 0;
const waiting: Array<() => void> = [];

async function limited<T>(task: () => Promise<T>): Promise<T> {
  if (running >= MAX_CONCURRENT) await new Promise<void>((resolve) => waiting.push(resolve));
  running += 1;
  try {
    return await task();
  } finally {
    running -= 1;
    waiting.shift()?.();
  }
}

/**
 * Asks the desktop host to grant the webview read access to one clip and its thumbnail
 * (generating the thumbnail if needed), then turns the paths into asset URLs.
 */
export function loadClipMedia(matchId: string, momentId: string): Promise<ClipMediaUrls> {
  if (!isTauri()) return Promise.resolve(none);
  const key = `${matchId}:${momentId}`;
  const cached = cache.get(key);
  if (cached) return cached;
  const request = limited(() => invoke<{ clipPath: string; thumbnailPath: string | null }>("prepare_clip_media", {
    matchId,
    momentId,
  }))
    .then((media) => ({
      clipUrl: convertFileSrc(media.clipPath),
      thumbnailUrl: media.thumbnailPath ? convertFileSrc(media.thumbnailPath) : null,
    }))
    .catch((error) => {
      // A missing or moved file must not be remembered as permanently unavailable.
      cache.delete(key);
      console.warn("Clip media is unavailable", error);
      return none;
    });
  cache.set(key, request);
  return request;
}

export async function setMomentFavorite(matchId: string, momentId: string, favorite: boolean): Promise<void> {
  if (isTauri()) await invoke("set_moment_favorite", { matchId, momentId, favorite });
}

export async function revealMomentClip(matchId: string, momentId: string): Promise<void> {
  if (isTauri()) await invoke("reveal_moment_clip", { matchId, momentId });
}
