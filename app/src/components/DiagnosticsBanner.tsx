import { useState } from "react";
import { Check, LifeBuoy, ShieldCheck, X } from "lucide-react";
import {
  dismissDiagnostics,
  submitDiagnostics,
  type PendingReport,
} from "../data/matchRepository";

interface DiagnosticsBannerProps {
  report: PendingReport;
  /** Turns off these prompts in Settings. */
  onNeverAsk: () => void;
  /** The report was sent or declined; remove the card. */
  onDone: () => void;
}

/**
 * Offered after a match when capture had a problem, as a card in the corner of the window. It states
 * plainly what would be sent, lets the user read all of it, and sends nothing until they press Send.
 */
export function DiagnosticsBanner({ report, onNeverAsk, onDone }: DiagnosticsBannerProps) {
  const [reviewing, setReviewing] = useState(false);
  const [note, setNote] = useState("");
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { payload } = report;

  const send = async () => {
    setSending(true);
    setError(null);
    try {
      await submitDiagnostics(note.trim() || null);
      setSent(true);
      window.setTimeout(onDone, 3_500);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSending(false);
    }
  };

  const decline = () => {
    void dismissDiagnostics().finally(onDone);
  };

  if (sent) {
    return (
      <aside aria-live="polite" className="oc-diag oc-diag-thanks" role="status">
        <span className="oc-diag-icon"><Check size={18} strokeWidth={2.6} /></span>
        <div>
          <strong>Report sent, thank you</strong>
          <p>It helps make OpenClip better for everyone.</p>
        </div>
      </aside>
    );
  }

  const gpu = payload.gpus.filter((adapter) => !adapter.software).map((adapter) => adapter.name)[0];
  const included = [
    `OpenClip ${payload.app_version}`,
    payload.os,
    gpu,
    payload.encoder,
    "Capture settings",
    "Anonymized log excerpt",
  ].filter(Boolean) as string[];

  return (
    <aside aria-label="Send an anonymous problem report" className="oc-diag" data-open={reviewing}>
      <header className="oc-diag-head">
        <span className="oc-diag-icon"><LifeBuoy size={18} /></span>
        <div>
          <strong>Recording ran into trouble</strong>
          <p>Send an anonymous report so it can be fixed?</p>
        </div>
        <button aria-label="Dismiss" className="oc-diag-close" disabled={sending} onClick={decline} type="button">
          <X size={15} />
        </button>
      </header>

      {payload.summary && <code className="oc-diag-summary">{payload.summary}</code>}

      {reviewing ? (
        <div className="oc-diag-review">
          <ul className="oc-diag-list">
            {included.map((item) => (
              <li key={item}><Check size={12} strokeWidth={3} />{item}</li>
            ))}
          </ul>
          <details>
            <summary>See exactly what is sent</summary>
            <pre>{JSON.stringify({ ...payload, user_note: note.trim() || null }, null, 2)}</pre>
          </details>
          <label className="oc-diag-note">
            <span>Anything to add? <em>Optional</em></span>
            <textarea
              maxLength={1000}
              onChange={(event) => setNote(event.target.value)}
              placeholder="What were you doing when it happened?"
              rows={2}
              value={note}
            />
          </label>
        </div>
      ) : (
        <p className="oc-diag-privacy">
          <ShieldCheck size={14} />
          No names, file paths or account details. Nothing is sent until you press Send.
        </p>
      )}

      {error && <div className="oc-diag-error" role="alert">Couldn’t send the report: {error}</div>}

      <footer className="oc-diag-actions">
        {reviewing ? (
          <button className="oc-btn primary" disabled={sending} onClick={() => void send()} type="button">
            {sending ? "Sending…" : "Send report"}
          </button>
        ) : (
          <button className="oc-btn primary" onClick={() => setReviewing(true)} type="button">Review and send</button>
        )}
        <button className="oc-btn ghost" disabled={sending} onClick={decline} type="button">Not now</button>
        <button className="oc-link" disabled={sending} onClick={() => { onNeverAsk(); decline(); }} type="button">
          Don’t ask again
        </button>
      </footer>
    </aside>
  );
}
