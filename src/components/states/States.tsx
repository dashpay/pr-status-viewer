// Full-pane and banner states for the main area: loading, load error,
// signed out, no projects, and the GitHub error banner.
import { useState, type ReactNode } from "react";
import { formatAgo, useNow } from "../shell/time";
import "./States.css";

/** Grey bars in the shape of the PR table, shown until the first snapshot. */
export function TableSkeleton() {
  const widths = [72, 54, 81, 63, 47, 69, 58, 76];
  return (
    <div className="skeleton" aria-busy="true" aria-live="polite">
      <span className="sr-only">Loading pull requests…</span>
      <div className="skeleton-tools" aria-hidden="true">
        <i style={{ width: 120 }} />
        <span className="skeleton-sp" />
        <i style={{ width: 170 }} />
      </div>
      {[0, 1].map((g) => (
        <div key={g} aria-hidden="true">
          <div className="skeleton-group">
            <i style={{ width: 130 }} />
            <i className="faint" style={{ width: 180 }} />
          </div>
          {widths.slice(g * 3, g * 3 + (g === 0 ? 5 : 3)).map((w, i) => (
            <div key={i} className="skeleton-row">
              <div className="skeleton-text">
                <i style={{ width: `${w}%` }} />
                <i className="faint small" style={{ width: `${Math.round(w / 2.4)}%` }} />
              </div>
              <div className="skeleton-bubbles">
                <span>{i % 2 === 0 && <i className="skeleton-pill" />}</span>
                <span>{i % 3 !== 2 && <i className="skeleton-pill" />}</span>
              </div>
              <i className="skeleton-eta" />
              <i className="skeleton-ci" />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

interface FullPaneProps {
  title: string;
  children?: ReactNode;
  actions?: ReactNode;
  tone?: "neutral" | "error";
}

/** A centred message that fills the main area. */
export function FullPane({ title, children, actions, tone = "neutral" }: FullPaneProps) {
  return (
    <div className="fullpane">
      <section className={`fullpane-card ${tone}`} aria-labelledby="fullpane-title">
        <h2 id="fullpane-title" className="fullpane-title">
          {tone === "error" && (
            <span className="fullpane-mark" aria-hidden="true">
              ⚠{" "}
            </span>
          )}
          {title}
        </h2>
        {children}
        {actions && <div className="fullpane-actions">{actions}</div>}
      </section>
    </div>
  );
}

/** A button that shows progress while its async action runs. */
export function AsyncButton({
  onClick,
  children,
  busyLabel,
  className = "btn",
}: {
  onClick(): Promise<unknown> | void;
  children: ReactNode;
  busyLabel: string;
  className?: string;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className={className}
      disabled={busy}
      aria-busy={busy || undefined}
      onClick={async () => {
        setBusy(true);
        try {
          await onClick();
        } finally {
          setBusy(false);
        }
      }}
    >
      {busy ? busyLabel : children}
    </button>
  );
}

export function LoadErrorState({ error, onRetry }: { error: string; onRetry(): Promise<unknown> }) {
  return (
    <FullPane
      title="Could not load your pull requests"
      tone="error"
      actions={
        <AsyncButton className="btn primary" onClick={onRetry} busyLabel="Trying again…">
          Try again
        </AsyncButton>
      }
    >
      <p className="fullpane-text selectable">{error}</p>
    </FullPane>
  );
}

export function SignedOutState({ error, onRetry }: { error: string | null; onRetry(): Promise<unknown> }) {
  return (
    <FullPane
      title="Sign in to GitHub"
      actions={
        <AsyncButton className="btn primary" onClick={onRetry} busyLabel="Checking…">
          Try again
        </AsyncButton>
      }
    >
      <p className="fullpane-text">PR Status needs a GitHub token. It finds one in either of two ways:</p>
      <ol className="fullpane-steps">
        <li>
          Run <code className="mono selectable">gh auth login</code> in a terminal. The app reads the GitHub CLI’s
          login.
        </li>
        <li>
          Or set <code className="mono selectable">GITHUB_TOKEN</code> in the environment before you launch the app.
        </li>
      </ol>
      {error && <p className="fullpane-text dim small selectable">{error}</p>}
    </FullPane>
  );
}

export function NoProjectsState({ onAdd }: { onAdd(): void }) {
  return (
    <FullPane
      title="Add a project to see its pull requests"
      actions={
        <button type="button" className="btn primary" onClick={onAdd}>
          + Add project
        </button>
      }
    >
      <p className="fullpane-text">
        A project is a GitHub repository. PR Status lists your open pull requests there, and any you follow.
      </p>
    </FullPane>
  );
}

/** Non-blocking notice above the table when the last GitHub sync failed. */
export function GithubErrorBanner({
  error,
  lastSuccessAt,
  onRetry,
}: {
  error: string | null;
  lastSuccessAt: string | null;
  onRetry(): Promise<unknown>;
}) {
  const now = useNow(1000);
  return (
    <div className="banner" role="status">
      <span className="banner-mark" aria-hidden="true">
        ⚠
      </span>
      <div className="banner-text">
        <strong>Could not reach GitHub.</strong> <span className="selectable">{error ?? "The last sync failed."}</span>{" "}
        <span className="dim">
          {lastSuccessAt ? `Last synced ${formatAgo(lastSuccessAt, now)}.` : "Not synced yet."}
        </span>
      </div>
      <AsyncButton onClick={onRetry} busyLabel="Retrying…">
        Retry
      </AsyncButton>
    </div>
  );
}
