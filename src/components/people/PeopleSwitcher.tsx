// The header's person switcher: whose pull requests are in view ("Mine", or
// a tracked contributor), with a menu to switch, track someone new, or stop
// tracking the contributor whose view is showing.
import { useRef, useState } from "react";
import { errorMessage, useApp } from "../../state/store";
import { contributorCount, sameLogin, viewCounts } from "../../lib/select";
import { openPullRequests } from "../../lib/people";
import { Menu, MenuItem, MenuSeparator } from "../ui/Menu";
import { TrackContributorDialog } from "./TrackContributorDialog";
import "./People.css";

const NO_ONE: string[] = [];

export function PeopleSwitcher() {
  const view = useApp((s) => s.view);
  const snapshot = useApp((s) => s.snapshot);
  const contributors = useApp((s) => s.config?.contributors ?? NO_ONE);
  const setView = useApp((s) => s.setView);
  const untrack = useApp((s) => s.untrackContributor);
  const [menuOpen, setMenuOpen] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const button = useRef<HTMLButtonElement>(null);

  // Following, Needs me and the projects are the user's own lists: still "Mine".
  const current = view.kind === "contributor" ? view.login : null;
  const label = current ?? "Mine";
  const counts = viewCounts(snapshot);

  const choose = (action: () => void) => {
    setMenuOpen(false);
    // Focus the button first, so a dialog opened next returns focus there.
    button.current?.focus();
    action();
  };

  const stopTracking = async (login: string) => {
    setError(null);
    try {
      await untrack(login);
    } catch (e) {
      setError(`Could not stop tracking ${login}: ${errorMessage(e)}`);
    }
  };

  return (
    <>
      <button
        ref={button}
        type="button"
        className={`people-switch${current ? " other" : ""}`}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label={`Whose pull requests: ${label}`}
        title={current ? `Showing ${label}'s pull requests` : "Showing your pull requests"}
        onClick={() => {
          setError(null);
          setMenuOpen((o) => !o);
        }}
      >
        <svg className="people-icon" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
          <circle cx="8" cy="5.2" r="2.7" fill="none" stroke="currentColor" strokeWidth="1.4" />
          <path d="M2.8 14c.6-2.9 2.7-4.4 5.2-4.4s4.6 1.5 5.2 4.4" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
        <span className="people-switch-label">{label}</span>
        <svg className="people-caret" viewBox="0 0 10 10" width="8" height="8" aria-hidden="true">
          <path d="M2 3.5 L5 6.5 L8 3.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {error && (
        <span className="people-error" role="alert" title={error}>
          <span aria-hidden="true">⚠ </span>
          {error}
        </span>
      )}

      {menuOpen && (
        <Menu
          anchor={button}
          label="Whose pull requests"
          caption="Show pull requests of"
          className="people-menu"
          onClose={(refocus) => {
            setMenuOpen(false);
            if (refocus) button.current?.focus();
          }}
        >
          <MenuItem checked={current === null} onSelect={() => choose(() => setView({ kind: "mine" }))}>
            <PersonRow name="Mine" count={counts.mine} />
          </MenuItem>
          {contributors.map((login) => (
            <MenuItem
              key={login}
              checked={current !== null && sameLogin(current, login)}
              title={login}
              onSelect={() => choose(() => setView({ kind: "contributor", login }))}
            >
              <PersonRow name={login} count={contributorCount(counts, login)} />
            </MenuItem>
          ))}
          <MenuSeparator />
          <MenuItem className="people-action" onSelect={() => choose(() => setDialogOpen(true))}>
            Track contributor…
          </MenuItem>
          {current && (
            <MenuItem
              danger
              className="people-action"
              title={`Stop tracking ${current}`} onSelect={() => choose(() => void stopTracking(current))}>
              Stop tracking {current}
            </MenuItem>
          )}
        </Menu>
      )}

      {dialogOpen && <TrackContributorDialog onClose={() => setDialogOpen(false)} />}
    </>
  );
}

function PersonRow({ name, count }: { name: string; count: number }) {
  return (
    <>
      <span className="people-name">{name}</span>
      <span className="people-n">
        {count}
        <span className="sr-only">, {openPullRequests(count)}</span>
      </span>
    </>
  );
}
