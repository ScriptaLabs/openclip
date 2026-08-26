import { ChevronDown, FolderOpen } from "lucide-react";

export function SettingsPage() {
  return (
    <div className="page settings-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">Keep it simple</span>
          <h1>Settings</h1>
          <p>Recording defaults designed for high-quality local clips.</p>
        </div>
      </header>

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

