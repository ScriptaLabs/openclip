import { useEffect, useState } from "react";
import { ChevronDown, FolderOpen } from "lucide-react";
import { getCs2SetupStatus, installCs2GsiConfig, type Cs2SetupStatus } from "../data/matchRepository";

export function SettingsPage() {
  const [setup, setSetup] = useState<Cs2SetupStatus | null>(null);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);

  const refreshSetup = () => getCs2SetupStatus().then(setSetup).catch((error) => setSetupError(String(error)));
  useEffect(() => { void refreshSetup(); }, []);

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
        <div>
          <h1>Settings</h1>
          <p>Recording defaults designed for high-quality local clips.</p>
        </div>
      </header>

      <section className="settings-panel integration-panel">
        <div className="settings-heading"><h2>Counter-Strike 2 integration</h2><p>{setupError ?? setup?.message ?? "Checking CS2 installation…"}</p></div>
        <button className="secondary-button" disabled={!setup?.found || setup.configured || installing} onClick={() => void install()} type="button">
          {setup?.configured ? "Configuration installed" : installing ? "Installing…" : "Install GSI configuration"}
        </button>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Recording</h2><p>Applied the next time capture starts.</p></div>
        <div className="settings-grid">
          <label><span>Quality</span><button type="button">High <ChevronDown size={15} /></button></label>
          <label><span>Resolution</span><button type="button">Game resolution <ChevronDown size={15} /></button></label>
          <label><span>Frame rate</span><button type="button">60 FPS <ChevronDown size={15} /></button></label>
        </div>
      </section>

      <section className="settings-panel">
        <div className="settings-heading"><h2>Automatic capture</h2><p>Choose which moments OpenClip should preserve.</p></div>
        <div className="toggle-list">
          {[["Kills", "Capture eliminations automatically"], ["Headshots", "Mark precision kills as highlights"], ["Multi-kills", "Merge nearby kills into one moment"]].map(([title, copy]) => (
            <label className="toggle-row" key={title}>
              <span><strong>{title}</strong><small>{copy}</small></span>
              <input type="checkbox" defaultChecked /><i />
            </label>
          ))}
        </div>
        <div className="settings-grid compact-settings">
          <label><span>Before event</span><button type="button">10 seconds <ChevronDown size={15} /></button></label>
          <label><span>After event</span><button type="button">5 seconds <ChevronDown size={15} /></button></label>
        </div>
      </section>

      <section className="settings-panel storage-panel">
        <div className="settings-heading"><h2>Storage</h2><p>Video stays on your machine.</p></div>
        <button className="path-button" type="button"><FolderOpen size={17} /><span>D:\OpenClip\Clips</span><em>Browse</em></button>
      </section>
    </div>
  );
}
