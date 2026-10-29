import capturedUrl from "../../assets/sfx/captured.mp3";
import errorUrl from "../../assets/sfx/error.mp3";

export type SoundKind = "captured" | "error";

const sources: Record<SoundKind, string> = { captured: capturedUrl, error: errorUrl };

/** Maps the 0-100 slider to amplitude; a squared curve feels linear to the ear. */
export function soundGain(volume: number): number {
  const level = Math.min(100, Math.max(0, volume)) / 100;
  return level * level;
}

export function playSound(kind: SoundKind, volume: number): void {
  const gain = soundGain(volume);
  if (gain <= 0) return;
  try {
    const audio = new Audio(sources[kind]);
    audio.volume = gain;
    void audio.play().catch(() => {});
  } catch {
    // Audio is a nicety; a missing output device must never break the overlay.
  }
}
