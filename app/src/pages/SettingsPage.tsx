import { useEffect, useState } from "react";
import { FolderOpen } from "lucide-react";
import {
  getCs2SetupStatus,
  getMicrophoneDevices,
  getSettings,
  installCs2GsiConfig,
  updateSettings,
  type AudioDevice,
  type Cs2SetupStatus,
  type RecordingSettings,
} from "../data/matchRepository";

export function SettingsPage() {
  const [setup, setSetup] = useState<Cs2SetupStatus | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [settings, setSettings] = useState<RecordingSettings | null>(null);
  const [microphones, setMicrophones] = useState<AudioDevice[]>([]);

  const refreshSetup = () => getCs2SetupStatus().then(setSetup).catch((error) => setSetupError(String(error)));
  useEffect(() => {
    void refreshSetup();
    void getSettings().then(setSettings).catch((error) => setSetupError(String(error)));
    void getMicrophoneDevices().then(setMicrophones).catch((error) => setSetupError(String(error)));
  }, []);

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
        <div className="settings-heading"><h2>Recording</h2><p>Changes are persisted and applied to the next encoded segment.</p></div>
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
        <div className="settings-heading"><h2>Microphone</h2><p>Choose the preference now; microphone mixing remains disabled in Milestone 3.</p></div>
        <div className="toggle-list">
          <label className="toggle-row"><span><strong>Capture microphone</strong><small>Saved for the upcoming microphone capture backend</small></span><input disabled={!settings} type="checkbox" checked={settings?.captureMicrophone ?? false} onChange={(event) => settings && save({ ...settings, captureMicrophone: event.target.checked })} /><i /></label>
        </div>
        <div className="settings-grid microphone-grid">
          <label><span>Input device</span><select disabled={!settings || !settings.captureMicrophone} value={settings?.microphoneDeviceId ?? ""} onChange={(event) => settings && save({ ...settings, microphoneDeviceId: event.target.value || null })}><option value="">Windows default microphone</option>{microphones.map((device) => <option key={device.id} value={device.id}>{device.name}</option>)}</select></label>
        </div>
      </section>

      <section className="settings-panel disabled-feature-panel">
        <div className="settings-heading"><h2>Automatic capture</h2><p>Planned for Milestone 4; these controls are intentionally disabled.</p></div>
        <div className="toggle-list">
          {[['Kills', 'Capture eliminations automatically'], ['Headshots', 'Mark precision kills as highlights'], ['Multi-kills', 'Merge nearby kills into one moment']].map(([title, copy]) => (
            <label className="toggle-row" key={title}><span><strong>{title}</strong><small>{copy}</small></span><input type="checkbox" checked={false} disabled readOnly /><i /></label>
          ))}
        </div>
        <div className="settings-grid compact-settings">
          <label><span>Before event</span><select disabled value={settings?.preRollSeconds ?? 10}><option>{settings?.preRollSeconds ?? 10} seconds</option></select></label>
          <label><span>After event</span><select disabled value={settings?.postRollSeconds ?? 5}><option>{settings?.postRollSeconds ?? 5} seconds</option></select></label>
        </div>
      </section>

      <section className="settings-panel storage-panel">
        <div className="settings-heading"><h2>Storage</h2><p>Video stays on your machine.</p></div>
        <div className="path-button"><FolderOpen size={17} /><span>{settings?.storagePath ?? "Loading…"}</span></div>
      </section>
    </div>
  );
}
