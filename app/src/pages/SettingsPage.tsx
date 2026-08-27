import { useEffect, useState, type KeyboardEvent } from "react";
import { FolderOpen } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  getCs2SetupStatus,
  getMicrophoneDevices,
  getSettings,
  installCs2GsiConfig,
  setStoragePath,
  updateSettings,
  type AudioDevice,
  type Cs2SetupStatus,
  type RecordingSettings,
} from "../data/matchRepository";

const displayStoragePath = (path?: string) => {
  if (!path) return "Loading…";
  return path.startsWith("\\\\?\\") ? path.slice(4) : path;
};

export function SettingsPage() {
  const [setup, setSetup] = useState<Cs2SetupStatus | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [settings, setSettings] = useState<RecordingSettings | null>(null);
  const [microphones, setMicrophones] = useState<AudioDevice[]>([]);
  const [shortcutDraft, setShortcutDraft] = useState("F8");
  const [savingShortcut, setSavingShortcut] = useState(false);

  const refreshSetup = () => getCs2SetupStatus().then(setSetup).catch((error) => setSetupError(String(error)));
  useEffect(() => {
    void refreshSetup();
    void getSettings().then((loaded) => { setSettings(loaded); setShortcutDraft(loaded.manualReplayShortcut); }).catch((error) => setSetupError(String(error)));
    void getMicrophoneDevices().then(setMicrophones).catch((error) => setSetupError(String(error)));
  }, []);

  useEffect(() => {
    if (settings) document.documentElement.dataset.accent = settings.accent;
  }, [settings?.accent]);

  const save = (next: RecordingSettings) => {
    setSettings(next);
    void updateSettings(next).catch((error) => setSetupError(String(error)));
  };

  const install = async () => {
    setInstalling(true);
    setSetupError(null);
    try {
      await installCs2GsiConfig();
      await refreshSetup();
    } catch (error) {
      setSetupError(error instanceof Error ? error.message : String(error));
    } finally {
      setInstalling(false);
    }
  };

  const chooseStorage = async () => {
    setSetupError(null);
    try {
      const selected = await open({ directory: true, multiple: false, title: "Choose OpenClip clip location" });
      if (selected) setSettings(await setStoragePath(selected));
    } catch (error) {
      setSetupError(error instanceof Error ? error.message : String(error));
    }
  };

  const captureShortcut = (event: KeyboardEvent<HTMLInputElement>) => {
    if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
    event.preventDefault();
    const modifiers = [event.ctrlKey && "Ctrl", event.altKey && "Alt", event.shiftKey && "Shift", event.metaKey && "Super"].filter(Boolean);
    const key = event.key === " " ? "Space" : event.key.length === 1 ? event.key.toUpperCase() : event.key;
    setShortcutDraft([...modifiers, key].join("+"));
  };

  const applyShortcut = async () => {
    if (!settings || shortcutDraft === settings.manualReplayShortcut) return;
    setSavingShortcut(true);
    setSetupError(null);
    const next = { ...settings, manualReplayShortcut: shortcutDraft };
    try {
      await updateSettings(next);
      setSettings(next);
    } catch (error) {
      setShortcutDraft(settings.manualReplayShortcut);
      setSetupError(error instanceof Error ? error.message : String(error));
    } finally {
      setSavingShortcut(false);
    }
  };

  return (
    <div className="page settings-page">
      <header className="page-header">
        <div><h1>Settings</h1><p>Recording defaults designed for high-quality local clips.</p></div>
      </header>

      <section className="settings-panel integration-panel">
        <div className="settings-heading"><h2>Counter-Strike 2 integration</h2><p>{setupError ?? setup?.message ?? "Checking CS2 installation…"}</p></div>
        <button className="secondary-button" disabled={!setup?.found || setup.configured || installing} onClick={() => void install()} type="button">
          {setup?.configured ? "Configuration installed" : installing ? "Installing…" : "Install GSI configuration"}
        </button>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Recording</h2><p>Capture changes restart the replay window so every saved clip stays compatible.</p></div>
        <div className="settings-grid">
          <label><span>Quality</span><select disabled={!settings} value={settings?.quality ?? "high"} onChange={(event) => settings && save({ ...settings, quality: event.target.value as RecordingSettings["quality"] })}><option value="performance">Performance</option><option value="high">High</option><option value="ultra">Ultra</option></select></label>
          <label><span>Resolution</span><select disabled={!settings} value={settings?.resolution ?? "game_resolution"} onChange={(event) => settings && save({ ...settings, resolution: event.target.value as RecordingSettings["resolution"] })}><option value="game_resolution">Game resolution</option><option value="p1080">1080p</option><option value="p1440">1440p</option></select></label>
          <label><span>Frame rate</span><select disabled={!settings} value={settings?.framesPerSecond ?? 60} onChange={(event) => settings && save({ ...settings, framesPerSecond: Number(event.target.value) })}><option value={30}>30 FPS</option><option value={60}>60 FPS</option></select></label>
        </div>
        <div className="toggle-list recording-toggles">
          <label className="toggle-row"><span><strong>Game audio</strong><small>Record audio from the CS2 process tree</small></span><input disabled={!settings} type="checkbox" checked={settings?.captureGameAudio ?? false} onChange={(event) => settings && save({ ...settings, captureGameAudio: event.target.checked })} /><i /></label>
        </div>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Microphone</h2><p>Mix an input device with CS2 audio in captured clips.</p></div>
        <div className="toggle-list">
          <label className="toggle-row"><span><strong>Capture microphone</strong><small>Recorded and mixed into the clip's AAC track</small></span><input disabled={!settings} type="checkbox" checked={settings?.captureMicrophone ?? false} onChange={(event) => settings && save({ ...settings, captureMicrophone: event.target.checked })} /><i /></label>
        </div>
        <div className="settings-grid microphone-grid">
          <label><span>Input device</span><select disabled={!settings || !settings.captureMicrophone} value={settings?.microphoneDeviceId ?? ""} onChange={(event) => settings && save({ ...settings, microphoneDeviceId: event.target.value || null })}><option value="">Windows default microphone</option>{microphones.map((device) => <option key={device.id} value={device.id}>{device.name}</option>)}</select></label>
        </div>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Automatic capture</h2><p>Nearby qualifying events merge until the post-roll window expires.</p></div>
        <div className="toggle-list">
          <label className="toggle-row"><span><strong>Kills</strong><small>Ordinary kills can create automatic moments</small></span><input disabled={!settings} type="checkbox" checked={settings?.captureKills ?? false} onChange={(event) => settings && save({ ...settings, captureKills: event.target.checked })} /><i /></label>
          <label className="toggle-row"><span><strong>Headshots</strong><small>Known GSI headshots receive extra importance</small></span><input disabled={!settings} type="checkbox" checked={settings?.captureHeadshots ?? false} onChange={(event) => settings && save({ ...settings, captureHeadshots: event.target.checked })} /><i /></label>
          <label className="toggle-row"><span><strong>Multi-kills</strong><small>Merge nearby kills within the same round</small></span><input disabled={!settings} type="checkbox" checked={settings?.captureMultiKills ?? false} onChange={(event) => settings && save({ ...settings, captureMultiKills: event.target.checked })} /><i /></label>
        </div>
        <div className="settings-grid compact-settings">
          <label><span>Before event</span><select disabled={!settings} value={settings?.preRollSeconds ?? 10} onChange={(event) => settings && save({ ...settings, preRollSeconds: Number(event.target.value) })}>{[5, 10, 15].map((seconds) => <option key={seconds} value={seconds}>{seconds} seconds</option>)}</select></label>
          <label><span>After event</span><select disabled={!settings} value={settings?.postRollSeconds ?? 5} onChange={(event) => settings && save({ ...settings, postRollSeconds: Number(event.target.value) })}>{[3, 5, 8, 10].map((seconds) => <option key={seconds} value={seconds}>{seconds} seconds</option>)}</select></label>
        </div>
      </section>

      <section className="settings-panel storage-panel">
        <div className="settings-heading"><h2>Storage</h2><p>Video stays on your machine.</p></div>
        <div className="storage-path-row"><div className="path-button"><FolderOpen size={17} /><span>{displayStoragePath(settings?.storagePath)}</span></div><button className="secondary-button" disabled={!settings} onClick={() => void chooseStorage()} type="button">Change folder</button></div>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Manual replay shortcut</h2><p>Works globally while OpenClip is running.</p></div>
        <div className="shortcut-setting">
          <label><span>Shortcut</span><input aria-label="Manual replay shortcut" disabled={!settings || savingShortcut} onKeyDown={captureShortcut} readOnly value={shortcutDraft} /></label>
          <button className="secondary-button" disabled={!settings || savingShortcut || shortcutDraft === settings.manualReplayShortcut} onClick={() => void applyShortcut()} type="button">{savingShortcut ? "Applying…" : "Apply"}</button>
        </div>
        <small className="settings-help">Click the field, then press a key or combination such as F8 or Ctrl+Shift+S.</small>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Appearance and behavior</h2><p>Restrained preferences for capture feedback and intentional accents.</p></div>
        <div className="settings-grid compact-settings">
          <label><span>Accent</span><select disabled={!settings} value={settings?.accent ?? "mint"} onChange={(event) => settings && save({ ...settings, accent: event.target.value as RecordingSettings["accent"] })}><option value="mint">Mint</option><option value="blue">Blue</option><option value="purple">Purple</option><option value="neutral">Neutral</option></select></label>
        </div>
        <div className="toggle-list recording-toggles">
          <label className="toggle-row"><span><strong>Clip captured notification</strong><small>Show a restrained in-app confirmation after automatic capture</small></span><input disabled={!settings} type="checkbox" checked={settings?.showClipNotifications ?? true} onChange={(event) => settings && save({ ...settings, showClipNotifications: event.target.checked })} /><i /></label>
          <label className="toggle-row"><span><strong>Animated export watermark</strong><small>Optional OpenClip mark in the top-right of exported clips</small></span><input disabled={!settings} type="checkbox" checked={settings?.watermarkEnabled ?? false} onChange={(event) => settings && save({ ...settings, watermarkEnabled: event.target.checked })} /><i /></label>
        </div>
      </section>
    </div>
  );
}
