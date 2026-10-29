import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { FolderOpen } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  getCs2SetupStatus,
  getMicrophoneDevices,
  getSettings,
  installCs2GsiConfig,
  previewOverlay,
  setStoragePath,
  updateSettings,
  type AudioDevice,
  type Cs2SetupStatus,
  type RecordingSettings,
  type RecordingStatus,
} from "../data/matchRepository";
import { checkForUpdate, getLaunchAtStartup, installUpdateNow, setLaunchAtStartup, useUpdateStatus, type UpdateStatus } from "../data/updates";
import { weaponIcon } from "../lib/weapons";
import { playSound } from "../overlay/sounds";
import mirage from "../../assets/map_splash/mirage.jpg";
import logo from "../../assets/branding/logo.png";

const displayPath = (path?: string) => {
  if (!path) return "Loading…";
  return path.startsWith("\\\\?\\") ? path.slice(4) : path;
};

function Setting({ label, help, disabled, children }: { label: string; help?: string; disabled?: boolean; children: ReactNode }) {
  return (
    <div className={`oc-setting ${disabled ? "off" : ""}`}>
      <div><strong>{label}</strong>{help && <small>{help}</small>}</div>
      {children}
    </div>
  );
}

function Switch({ label, checked, disabled, onChange }: { label: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return (
    <button aria-checked={checked} aria-label={label} className="oc-switch" disabled={disabled} onClick={() => onChange(!checked)} role="switch" type="button">
      <i />
    </button>
  );
}

function Segmented<T extends string | number>({ label, value, options, disabled, onChange }: {
  label: string;
  value: T;
  options: Array<{ value: T; label: string }>;
  disabled?: boolean;
  onChange: (value: T) => void;
}) {
  return (
    <div aria-label={label} className="oc-seg" role="group">
      {options.map((option) => (
        <button
          aria-pressed={option.value === value}
          className={option.value === value ? "active" : ""}
          disabled={disabled}
          key={String(option.value)}
          onClick={() => onChange(option.value)}
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

const SECTIONS = [
  { id: "capture", label: "Capture" },
  { id: "moments", label: "Moments" },
  { id: "overlay", label: "Overlay and sounds" },
  { id: "storage", label: "Storage" },
  { id: "shortcuts", label: "Shortcuts" },
  { id: "startup", label: "Startup and updates" },
  { id: "game", label: "Counter-Strike 2" },
  { id: "appearance", label: "Appearance" },
] as const;

function updateSummary(status: UpdateStatus): string {
  if (status.error) return `Couldn’t check for updates: ${status.error}`;
  switch (status.phase) {
    case "checking": return "Checking for updates…";
    case "downloading": return `Downloading v${status.availableVersion}…`;
    case "ready": return `v${status.availableVersion} is ready. It installs when your match ends.`;
    case "available": return `v${status.availableVersion} is available.`;
    case "installing": return "Restarting to finish the update…";
    default:
      return status.lastChecked
        ? `You’re up to date. Last checked ${new Date(status.lastChecked).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}.`
        : "OpenClip checks for updates automatically.";
  }
}

export function SettingsPage({ recordingStatus }: { recordingStatus: RecordingStatus | null }) {
  const [setup, setSetup] = useState<Cs2SetupStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [settings, setSettings] = useState<RecordingSettings | null>(null);
  const [microphones, setMicrophones] = useState<AudioDevice[]>([]);
  const [listening, setListening] = useState(false);
  const [active, setActive] = useState<string>(SECTIONS[0].id);
  const [launchAtStartup, setLaunchAtStartupState] = useState<boolean | null>(null);
  const update = useUpdateStatus();
  const refs = useRef(new Map<string, HTMLElement>());

  const refreshSetup = () => getCs2SetupStatus().then(setSetup).catch((failure) => setError(String(failure)));
  useEffect(() => {
    void refreshSetup();
    void getSettings().then(setSettings).catch((failure) => setError(String(failure)));
    void getLaunchAtStartup().then(setLaunchAtStartupState).catch(() => setLaunchAtStartupState(false));
    void getMicrophoneDevices().then(setMicrophones).catch((failure) => setError(String(failure)));
  }, []);

  useEffect(() => {
    if (settings) document.documentElement.dataset.accent = settings.accent;
  }, [settings?.accent]);

  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((entry) => entry.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
        if (visible) setActive(visible.target.id);
      },
      { rootMargin: "-20% 0px -65% 0px" },
    );
    refs.current.forEach((node) => observer.observe(node));
    return () => observer.disconnect();
  }, []);

  const save = (next: RecordingSettings) => {
    setSettings(next);
    updateSettings(next).catch((failure) => setError(failure instanceof Error ? failure.message : String(failure)));
  };

  // The slider moves local state while dragging; the value is stored (and heard) on release.
  const commitVolume = (audible: boolean) => {
    if (!settings) return;
    updateSettings(settings).catch((failure) => setError(String(failure)));
    if (audible) playSound("captured", settings.soundVolume);
  };

  const install = async () => {
    setInstalling(true);
    setError(null);
    try {
      await installCs2GsiConfig();
      await refreshSetup();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setInstalling(false);
    }
  };

  const chooseStorage = async () => {
    setError(null);
    try {
      const selected = await open({ directory: true, multiple: false, title: "Choose OpenClip clip location" });
      if (selected) setSettings(await setStoragePath(selected));
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    }
  };

  useEffect(() => {
    if (!listening || !settings) return;
    const onKey = async (event: KeyboardEvent) => {
      if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
      event.preventDefault();
      if (event.key === "Escape") {
        setListening(false);
        return;
      }
      const modifiers = [event.ctrlKey && "Ctrl", event.altKey && "Alt", event.shiftKey && "Shift", event.metaKey && "Super"].filter(Boolean);
      const key = event.key === " " ? "Space" : event.key.length === 1 ? event.key.toUpperCase() : event.key;
      const next = { ...settings, manualReplayShortcut: [...modifiers, key].join("+") };
      setListening(false);
      try {
        await updateSettings(next);
        setSettings(next);
      } catch (failure) {
        setError(failure instanceof Error ? failure.message : String(failure));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [listening, settings]);

  const s = settings;
  const overlayOff = !s?.showClipNotifications;
  const ak = weaponIcon("weapon_ak47");

  return (
    <div className="oc-page">
      <header className="oc-head">
        <div>
          <h1>Settings</h1>
          <p>Changes apply immediately. Capture changes restart the recorder.</p>
        </div>
      </header>
      {error && <div className="oc-error" role="alert">{error}</div>}

      <div className="oc-settings">
        <nav aria-label="Settings sections" className="oc-subnav">
          {SECTIONS.map((section) => (
            <button
              className={active === section.id ? "active" : ""}
              key={section.id}
              onClick={() => {
                setActive(section.id);
                refs.current.get(section.id)?.scrollIntoView({ behavior: "smooth", block: "start" });
              }}
              type="button"
            >
              {section.label}
            </button>
          ))}
        </nav>

        <div className="oc-sections">
          <section className="oc-card oc-section" id="capture" ref={(node) => { if (node) refs.current.set("capture", node); }}>
            <h2>Capture</h2>
            <p>How OpenClip records CS2. Hardware encoding runs on your graphics card.</p>
            <Setting label="Quality" help="Higher quality uses more disk space per clip.">
              <Segmented disabled={!s} label="Quality" onChange={(quality) => s && save({ ...s, quality })} options={[{ value: "performance", label: "Performance" }, { value: "high", label: "High" }, { value: "ultra", label: "Ultra" }]} value={s?.quality ?? "high"} />
            </Setting>
            <Setting label="Resolution" help="Match the game, or scale clips down.">
              <select aria-label="Resolution" className="oc-select" disabled={!s} onChange={(event) => s && save({ ...s, resolution: event.target.value as RecordingSettings["resolution"] })} value={s?.resolution ?? "game_resolution"}>
                <option value="game_resolution">Game resolution</option><option value="p1080">1080p</option><option value="p1440">1440p</option>
              </select>
            </Setting>
            <Setting label="Frame rate" help="Clips are recorded at a steady frame rate.">
              <Segmented disabled={!s} label="Frame rate" onChange={(framesPerSecond) => s && save({ ...s, framesPerSecond })} options={[{ value: 30, label: "30 fps" }, { value: 60, label: "60 fps" }]} value={s?.framesPerSecond ?? 60} />
            </Setting>
            <Setting label="Encoder" help={`Auto uses your graphics card (NVIDIA, Intel, AMD and others) and falls back to the CPU.${recordingStatus?.encoder ? ` Recording with ${recordingStatus.encoder}.` : ""}`}>
              <select aria-label="Encoder" className="oc-select" disabled={!s} onChange={(event) => s && save({ ...s, videoEncoder: event.target.value as RecordingSettings["videoEncoder"] })} value={s?.videoEncoder ?? "auto"}>
                <option value="auto">Auto</option><option value="hardware">Graphics card only</option><option value="software">CPU only</option>
              </select>
            </Setting>
            <Setting label="Game audio" help="Record sound from the CS2 process.">
              <Switch checked={s?.captureGameAudio ?? false} disabled={!s} label="Game audio" onChange={(captureGameAudio) => s && save({ ...s, captureGameAudio })} />
            </Setting>
            <Setting label="Microphone" help="Mix your microphone into the clip's audio track.">
              <Switch checked={s?.captureMicrophone ?? false} disabled={!s} label="Microphone" onChange={(captureMicrophone) => s && save({ ...s, captureMicrophone })} />
            </Setting>
            <Setting disabled={!s?.captureMicrophone} label="Input device" help="Which microphone to record.">
              <select aria-label="Input device" className="oc-select" disabled={!s || !s.captureMicrophone} onChange={(event) => s && save({ ...s, microphoneDeviceId: event.target.value || null })} value={s?.microphoneDeviceId ?? ""}>
                <option value="">Windows default microphone</option>
                {microphones.map((device) => <option key={device.id} value={device.id}>{device.name}</option>)}
              </select>
            </Setting>
          </section>

          <section className="oc-card oc-section" id="moments" ref={(node) => { if (node) refs.current.set("moments", node); }}>
            <h2>Moments</h2>
            <p>Which plays become clips, and how much context each one includes.</p>
            <Setting label="Kills" help="Create a clip for ordinary kills.">
              <Switch checked={s?.captureKills ?? false} disabled={!s} label="Kills" onChange={(captureKills) => s && save({ ...s, captureKills })} />
            </Setting>
            <Setting label="Headshots" help="Known headshots score higher.">
              <Switch checked={s?.captureHeadshots ?? false} disabled={!s} label="Headshots" onChange={(captureHeadshots) => s && save({ ...s, captureHeadshots })} />
            </Setting>
            <Setting label="Multi-kills" help="Merge nearby kills in the same round into one clip.">
              <Switch checked={s?.captureMultiKills ?? false} disabled={!s} label="Multi-kills" onChange={(captureMultiKills) => s && save({ ...s, captureMultiKills })} />
            </Setting>
            <Setting disabled={!s?.captureMultiKills} label="Multi-kill window" help="Kills closer together than this count as one multi-kill. A longer window groups slower double and triple kills.">
              <Segmented disabled={!s || !s.captureMultiKills} label="Multi-kill window" onChange={(multiKillWindowSeconds) => s && save({ ...s, multiKillWindowSeconds })} options={[5, 8, 12, 15].map((value) => ({ value, label: `${value} s` }))} value={s?.multiKillWindowSeconds ?? 8} />
            </Setting>
            <Setting label="Before the kill" help="Footage kept ahead of the first kill.">
              <Segmented disabled={!s} label="Before the kill" onChange={(preRollSeconds) => s && save({ ...s, preRollSeconds })} options={[5, 10, 15].map((value) => ({ value, label: `${value} s` }))} value={s?.preRollSeconds ?? 10} />
            </Setting>
            <Setting label="After the kill" help="Footage kept after the last kill.">
              <Segmented disabled={!s} label="After the kill" onChange={(postRollSeconds) => s && save({ ...s, postRollSeconds })} options={[3, 5, 8, 10].map((value) => ({ value, label: `${value} s` }))} value={s?.postRollSeconds ?? 5} />
            </Setting>
          </section>

          <section className="oc-card oc-section" id="overlay" ref={(node) => { if (node) refs.current.set("overlay", node); }}>
            <h2>Overlay and sounds</h2>
            <p>A confirmation after each moment is saved. It never appears during the play, so it is never in your footage. It works over borderless and windowed CS2, not exclusive fullscreen.</p>
            <Setting label="Capture overlay" help="Tells you whether a play was captured, partial or missed.">
              <Switch checked={s?.showClipNotifications ?? true} disabled={!s} label="Capture overlay" onChange={(showClipNotifications) => s && save({ ...s, showClipNotifications })} />
            </Setting>
            <Setting disabled={overlayOff} label="Position" help="Bottom corners are not offered because they cover your health, armor and ammo.">
              <Segmented disabled={!s || overlayOff} label="Overlay position" onChange={(overlayPosition) => s && save({ ...s, overlayPosition })} options={[{ value: "top_left", label: "Top left" }, { value: "top_right", label: "Top right" }]} value={s?.overlayPosition ?? "top_left"} />
            </Setting>
            <Setting disabled={overlayOff} label="Clip preview" help="Plays the last few seconds of the saved clip under the confirmation.">
              <Switch checked={s?.overlayClipPreview ?? true} disabled={!s || overlayOff} label="Clip preview" onChange={(overlayClipPreview) => s && save({ ...s, overlayClipPreview })} />
            </Setting>
            <Setting label="Notification sounds" help="Turn off to keep OpenClip completely silent.">
              <Switch checked={s?.soundsEnabled ?? true} disabled={!s} label="Notification sounds" onChange={(soundsEnabled) => s && save({ ...s, soundsEnabled })} />
            </Setting>
            <Setting disabled={!s?.soundsEnabled} label="Volume" help="Applies to the captured and error sounds.">
              <div className="oc-slider">
                <input
                  aria-label="Notification volume"
                  disabled={!s || !s.soundsEnabled}
                  max={100}
                  min={0}
                  onBlur={() => commitVolume(false)}
                  onChange={(event) => s && setSettings({ ...s, soundVolume: Number(event.target.value) })}
                  onKeyUp={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End") commitVolume(true); }}
                  onPointerUp={() => commitVolume(true)}
                  step={5}
                  style={{ "--fill": `${s?.soundVolume ?? 60}%` } as CSSProperties}
                  type="range"
                  value={s?.soundVolume ?? 60}
                />
                <output>{s?.soundVolume ?? 60}%</output>
              </div>
            </Setting>
            <div className="oc-preview">
              <div className="oc-preview-head">
                <strong>Preview</strong>
                <div className="oc-actions">
                  <button className="oc-btn small" disabled={!s} onClick={() => void previewOverlay("captured")} type="button">Test captured</button>
                  <button className="oc-btn small" disabled={!s} onClick={() => void previewOverlay("partial")} type="button">Partial</button>
                  <button className="oc-btn small" disabled={!s} onClick={() => void previewOverlay("failed")} type="button">Failed</button>
                  <button className="oc-btn small" disabled={!s || !s.soundsEnabled} onClick={() => s && playSound("captured", s.soundVolume)} type="button">Test sound</button>
                </div>
              </div>
              <div className="oc-screen">
                <img alt="" src={mirage} />
                <span className="oc-zone" style={{ left: 14 }}>Health and armor</span>
                <span className="oc-zone" style={{ right: 14 }}>Ammo</span>
                <div className={`oc-mini ${s?.overlayPosition === "top_right" ? "right" : "left"}`} style={{ opacity: overlayOff ? 0.35 : 1 }}>
                  <span className="logo"><img alt="" src={logo} /></span>
                  <div>
                    <em>Clip captured</em>
                    <strong>Quad Kill</strong>
                    <small>4 kills {ak && <img alt="AK-47" src={ak} />}</small>
                  </div>
                </div>
              </div>
            </div>
          </section>

          <section className="oc-card oc-section" id="storage" ref={(node) => { if (node) refs.current.set("storage", node); }}>
            <h2>Storage</h2>
            <p>Video stays on your computer. Nothing is uploaded.</p>
            <Setting label="Clip folder" help="New clips are saved here. Existing clips stay where they are.">
              <div className="oc-path">
                <span title={displayPath(s?.storagePath)}><FolderOpen size={16} />{displayPath(s?.storagePath)}</span>
                <button className="oc-btn" disabled={!s} onClick={() => void chooseStorage()} type="button">Change</button>
              </div>
            </Setting>
          </section>

          <section className="oc-card oc-section" id="shortcuts" ref={(node) => { if (node) refs.current.set("shortcuts", node); }}>
            <h2>Shortcuts</h2>
            <p>Works anywhere in Windows while OpenClip is running.</p>
            <Setting label="Save replay now" help="Saves the last 15 seconds as a manual clip.">
              <div className="oc-path">
                <kbd className={`oc-key ${listening ? "listening" : ""}`}>{listening ? "Press keys…" : s?.manualReplayShortcut ?? "F8"}</kbd>
                <button className="oc-btn" disabled={!s} onClick={() => setListening((value) => !value)} type="button">{listening ? "Cancel" : "Change"}</button>
              </div>
            </Setting>
          </section>

          <section className="oc-card oc-section" id="startup" ref={(node) => { if (node) refs.current.set("startup", node); }}>
            <h2>Startup and updates</h2>
            <p>OpenClip only records while it is running, so it can start with Windows and stay out of your way in the tray.</p>
            <Setting label="Launch with Windows" help="Starts quietly in the tray when you sign in.">
              <Switch
                checked={launchAtStartup ?? false}
                disabled={launchAtStartup === null}
                label="Launch with Windows"
                onChange={(enabled) => {
                  setLaunchAtStartupState(enabled);
                  setLaunchAtStartup(enabled).catch((failure) => {
                    setLaunchAtStartupState(!enabled);
                    setError(failure instanceof Error ? failure.message : String(failure));
                  });
                }}
              />
            </Setting>
            <Setting label="Keep recording when closed" help="Closing the window sends OpenClip to the tray. Quit from the tray icon.">
              <Switch checked={s?.closeToTray ?? true} disabled={!s} label="Keep recording when closed" onChange={(closeToTray) => s && save({ ...s, closeToTray })} />
            </Setting>
            <Setting label="Automatic updates" help="New versions download in the background and install between matches, never during one.">
              <Switch checked={s?.autoUpdate ?? true} disabled={!s} label="Automatic updates" onChange={(autoUpdate) => s && save({ ...s, autoUpdate })} />
            </Setting>
            <Setting label={`Version ${update.currentVersion}`} help={updateSummary(update)}>
              <div className="oc-path">
                {update.phase === "ready" && <button className="oc-btn primary" onClick={() => void installUpdateNow()} type="button">Restart to update</button>}
                <button className="oc-btn" disabled={["checking", "downloading", "installing"].includes(update.phase)} onClick={() => void checkForUpdate()} type="button">Check for updates</button>
              </div>
            </Setting>
          </section>

          <section className="oc-card oc-section" id="game" ref={(node) => { if (node) refs.current.set("game", node); }}>
            <h2>Counter-Strike 2</h2>
            <p>OpenClip reads game events through CS2 Game State Integration.</p>
            <Setting label="Game integration" help={setup?.message ?? "Checking your CS2 installation…"}>
              <div className="oc-path">
                {setup && (
                  <span className={`oc-badge ${setup.configured ? "" : "warn"}`} style={{ border: 0, padding: "0 12px", background: undefined, maxWidth: "none" }}>
                    <i />{setup.configured ? "Connected" : setup.found ? "Not installed" : "CS2 not found"}
                  </span>
                )}
                <button className="oc-btn" disabled={!setup?.found || setup.configured || installing} onClick={() => void install()} type="button">
                  {installing ? "Installing…" : setup?.configured ? "Installed" : "Install"}
                </button>
              </div>
            </Setting>
          </section>

          <section className="oc-card oc-section" id="appearance" ref={(node) => { if (node) refs.current.set("appearance", node); }}>
            <h2>Appearance</h2>
            <p>Small touches. The app stays dark.</p>
            <Setting label="Accent color" help="Used for highlights, markers and status.">
              <div className="oc-swatches">
                {([["mint", "#72d5a8"], ["blue", "#74aef5"], ["purple", "#b79af4"], ["neutral", "#c7cbd2"]] as const).map(([value, color]) => (
                  <button
                    aria-label={value}
                    aria-pressed={s?.accent === value}
                    className={s?.accent === value ? "active" : ""}
                    key={value}
                    onClick={() => s && save({ ...s, accent: value })}
                    style={{ background: color }}
                    type="button"
                  />
                ))}
              </div>
            </Setting>
            <Setting label="Export watermark" help="Adds the OpenClip mark to the top right of exported clips.">
              <Switch checked={s?.watermarkEnabled ?? false} disabled={!s} label="Export watermark" onChange={(watermarkEnabled) => s && save({ ...s, watermarkEnabled })} />
            </Setting>
          </section>
        </div>
      </div>
    </div>
  );
}
