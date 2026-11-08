import { useEffect, useState } from "react";
import { Star } from "lucide-react";
import {
  applyRetention,
  previewRetention,
  type RecordingSettings,
  type RetentionPreview,
} from "../data/matchRepository";

const AGE_CHOICES = [7, 14, 30, 60, 90, 180];
const SIZE_CHOICES = [10, 25, 50, 100, 250, 500];

function gigabytes(bytes: number): string {
  const value = bytes / 2 ** 30;
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} GB`;
}

interface Props {
  settings: RecordingSettings | null;
  /** Saves the settings, resolving once they are stored. */
  onSave: (next: RecordingSettings) => Promise<void>;
}

/**
 * Automatic cleanup of old clips. The limits are a draft until applied, and the card says what applying
 * them would delete first, because deleting video cannot be undone. Favorites are never deleted.
 */
export function RetentionSettings({ settings, onSave }: Props) {
  const [age, setAge] = useState<number | null>(null);
  const [size, setSize] = useState<number | null>(null);
  const [preview, setPreview] = useState<RetentionPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const savedAge = settings?.retentionMaxAgeDays ?? null;
  const savedSize = settings?.retentionMaxSizeGb ?? null;
  useEffect(() => {
    setAge(savedAge);
    setSize(savedSize);
  }, [savedAge, savedSize]);

  useEffect(() => {
    if (!settings) return;
    let active = true;
    void previewRetention(age, size).then((result) => { if (active) setPreview(result); }).catch(() => {});
    return () => { active = false; };
  }, [settings, age, size]);

  const dirty = age !== savedAge || size !== savedSize;
  const toDelete = preview?.deleteClips ?? 0;
  const active = age !== null || size !== null;

  const apply = async () => {
    if (!settings) return;
    setBusy(true);
    setMessage(null);
    try {
      await onSave({ ...settings, retentionMaxAgeDays: age, retentionMaxSizeGb: size });
      const result = await applyRetention();
      setMessage(result.deleted > 0
        ? `Deleted ${result.deleted} ${result.deleted === 1 ? "clip" : "clips"} (${gigabytes(result.freedBytes)}).`
        : "Saved. Nothing needed deleting.");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="oc-retention">
      <div className="oc-retention-head">
        <strong>Clean up old clips</strong>
        <small>Free disk space automatically. Favorite clips are always kept.</small>
      </div>
      <div className="oc-retention-usage">
        {preview
          ? <><b>{gigabytes(preview.totalBytes)}</b> in {preview.totalClips} {preview.totalClips === 1 ? "clip" : "clips"}
            {preview.favoriteClips > 0 && <> · <Star fill="currentColor" size={12} /> {preview.favoriteClips} kept as {preview.favoriteClips === 1 ? "a favorite" : "favorites"}</>}</>
          : "Measuring clips…"}
      </div>
      <div className="oc-retention-row">
        <label>
          <span>Delete clips older than</span>
          <select className="oc-select" disabled={!settings || busy} onChange={(event) => setAge(event.target.value ? Number(event.target.value) : null)} value={age ?? ""}>
            <option value="">Never</option>
            {AGE_CHOICES.map((days) => <option key={days} value={days}>{days} days</option>)}
          </select>
        </label>
        <label>
          <span>Delete the oldest once clips pass</span>
          <select className="oc-select" disabled={!settings || busy} onChange={(event) => setSize(event.target.value ? Number(event.target.value) : null)} value={size ?? ""}>
            <option value="">No limit</option>
            {SIZE_CHOICES.map((gb) => <option key={gb} value={gb}>{gb} GB</option>)}
          </select>
        </label>
      </div>
      <p className="oc-retention-note">
        <Star fill="currentColor" size={12} /> Favorites are never deleted, and neither are clips from the last few minutes.
        The moment stays in your match history, without its video.
      </p>
      {dirty && active && toDelete > 0 && (
        <p className="oc-retention-warn">
          Applying this deletes {toDelete} {toDelete === 1 ? "clip" : "clips"} ({gigabytes(preview?.deleteBytes ?? 0)}) from your computer now. This can’t be undone.
        </p>
      )}
      {!dirty && active && toDelete > 0 && (
        <p className="oc-retention-warn">{toDelete} {toDelete === 1 ? "clip is" : "clips are"} over the limit and will be deleted at the next cleanup.</p>
      )}
      <div className="oc-retention-actions">
        {dirty && (
          <button className={`oc-btn ${toDelete > 0 && active ? "danger" : "primary"}`} disabled={busy || !settings} onClick={() => void apply()} type="button">
            {busy ? "Working…" : toDelete > 0 && active ? `Save and delete ${toDelete} ${toDelete === 1 ? "clip" : "clips"}` : "Save"}
          </button>
        )}
        {dirty && <button className="oc-btn ghost" disabled={busy} onClick={() => { setAge(savedAge); setSize(savedSize); }} type="button">Cancel</button>}
        {message && <span className="oc-retention-message" role="status">{message}</span>}
      </div>
    </div>
  );
}
