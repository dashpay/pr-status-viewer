// Track a contributor: type a GitHub username, or pick one of the people with
// open pull requests in the tracked projects. Typing also narrows that list.
// On success the store has already switched to that person's view.
import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";
import { errorMessage, useApp } from "../../state/store";
import type { KnownContributor } from "../../types";
import { filterKnown, openPullRequests, typedLogin } from "../../lib/people";
import { Dialog } from "../ui/Dialog";
import "./People.css";

type ListState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; people: KnownContributor[] };

export function TrackContributorDialog({ onClose }: { onClose(): void }) {
  const track = useApp((s) => s.trackContributor);
  const knownContributors = useApp((s) => s.knownContributors);

  const [value, setValue] = useState("");
  /** The login being tracked, or "" for what was typed; null when idle. */
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [list, setList] = useState<ListState>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const fieldId = useId();
  const errorId = useId();
  const listHeadId = useId();

  useEffect(() => {
    let live = true;
    setList({ status: "loading" });
    knownContributors().then(
      (people) => live && setList({ status: "ready", people }),
      (e) => live && setList({ status: "error", message: errorMessage(e) }),
    );
    return () => {
      live = false;
    };
  }, [knownContributors, attempt]);

  const visible = useMemo(() => (list.status === "ready" ? filterKnown(list.people, value) : []), [list, value]);

  const run = async (login: string, fromList: boolean) => {
    if (busy !== null) return;
    setBusy(fromList ? login : "");
    setError(null);
    try {
      await track(login);
      onClose();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(null);
      input.current?.focus();
    }
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!value.trim()) {
      setError("Type a GitHub username, or choose someone below.");
      input.current?.focus();
      return;
    }
    void run(value.trim(), false);
  };

  const typed = typedLogin(value);

  return (
    <Dialog
      title="Track contributor"
      description="See someone's open pull requests in the projects you track."
      onClose={onClose}
      initialFocus={input}
    >
      <form className="dialog-body track-form" onSubmit={submit}>
        <label className="dialog-label" htmlFor={fieldId}>
          GitHub username
        </label>
        <div className="track-row">
          <input
            ref={input}
            id={fieldId}
            className="input mono"
            placeholder="login, @login or github.com/login"
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={value}
            onChange={(e) => {
              setValue(e.target.value);
              if (error) setError(null);
            }}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
          />
          <button type="submit" className="btn primary" disabled={busy !== null} aria-busy={busy === "" || undefined}>
            {busy === "" ? "Tracking…" : "Track"}
          </button>
        </div>
        {error && (
          <p id={errorId} className="dialog-error" role="alert">
            {error}
          </p>
        )}
      </form>

      <section className="track-known" aria-labelledby={listHeadId} aria-busy={list.status === "loading"}>
        <h3 id={listHeadId} className="track-known-head">
          People with open pull requests in your projects
        </h3>
        {list.status === "loading" && <p className="track-state dim">Looking for people…</p>}
        {list.status === "error" && (
          <div className="track-state">
            <p className="dialog-error" role="alert">
              Could not load the people in your projects: {list.message}
            </p>
            <button type="button" className="btn" onClick={() => setAttempt((n) => n + 1)}>
              Try again
            </button>
          </div>
        )}
        {list.status === "ready" && list.people.length === 0 && (
          <p className="track-state dim">Nobody else has open pull requests in your projects</p>
        )}
        {list.status === "ready" && list.people.length > 0 && visible.length === 0 && (
          <p className="track-state dim">Nobody here matches “{typed}”</p>
        )}
        {visible.length > 0 && (
          <ul className="track-people">
            {visible.map((p) => (
              <li key={p.login}>
                <button
                  type="button"
                  className="track-person"
                  disabled={busy !== null}
                  aria-busy={busy === p.login || undefined}
                  aria-label={`Track ${p.login}, ${openPullRequests(p.openPrs)}`}
                  title={`Track ${p.login}`}
                  onClick={() => void run(p.login, true)}
                >
                  <Avatar login={p.login} url={p.avatarUrl} />
                  <span className="track-person-login mono">{p.login}</span>
                  <span className="track-person-n dim">
                    {busy === p.login ? "Tracking…" : openPullRequests(p.openPrs)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <div className="dialog-actions">
        <button type="button" className="btn" onClick={onClose}>
          Cancel
        </button>
      </div>
    </Dialog>
  );
}

/** The GitHub avatar, or the login's first letter when there is none (or it fails to load). */
function Avatar({ login, url }: { login: string; url: string | null }) {
  const [failed, setFailed] = useState(false);
  if (url && !failed) {
    return <img className="track-avatar" src={url} alt="" width={20} height={20} onError={() => setFailed(true)} />;
  }
  return (
    <span className="track-avatar letter" aria-hidden="true">
      {login.slice(0, 1).toUpperCase()}
    </span>
  );
}
