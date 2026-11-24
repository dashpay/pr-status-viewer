// A project's area switches, shown under its sidebar row when opened: one
// checkbox per area (the scope in a PR title) among the project's listed
// pull requests. Unticking an area hides its pull requests in every view.

import { useEffect, useRef, type KeyboardEvent } from "react";
import { NO_AREA, NO_AREA_LABEL, type AreaCount } from "../../lib/areas";
import "./AreaFilter.css";

export interface AreaFilterProps {
  id: string;
  repo: string;
  /** Areas present among the project's listed pull requests. */
  areas: AreaCount[];
  /** Area keys switched off for this project, present or not. */
  hidden: readonly string[];
  /** Scroll the panel into the sidebar's view when it mounts. */
  scrollOnOpen: boolean;
  onToggle(key: string): void;
  onAll(): void;
  onNone(): void;
  /** Escape inside the panel: close it and return focus to its toggle. */
  onClose(): void;
}

const prsWord = (n: number) => (n === 1 ? "pull request" : "pull requests");

export function AreaFilter({ id, repo, areas, hidden, scrollOnOpen, onToggle, onAll, onNone, onClose }: AreaFilterProps) {
  const panel = useRef<HTMLDivElement>(null);
  const present = new Set(areas.map((a) => a.key));
  // Switched off, but no listed pull request names it any more. Kept, so it
  // stays off if one comes back; "All" clears it.
  const absent = hidden.filter((k) => !present.has(k));
  const allOff = areas.length > 0 && areas.every((a) => hidden.includes(a.key));
  const showActions = areas.length >= 3 || absent.length > 0;

  useEffect(() => {
    if (scrollOnOpen) panel.current?.scrollIntoView?.({ block: "nearest" });
    // Mount only: later re-renders must not pull the list around.
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  return (
    <div ref={panel} id={id} className="areas" role="group" aria-label={`Areas in ${repo}`} onKeyDown={onKeyDown}>
      {showActions && (
        <div className="areas__actions">
          <span className="areas__actions-label dim">Show</span>
          <button
            type="button"
            className="areas__action"
            disabled={hidden.length === 0}
            aria-label={`Show all areas in ${repo}`}
            onClick={onAll}
          >
            All
          </button>
          <button
            type="button"
            className="areas__action"
            disabled={areas.length === 0 || allOff}
            aria-label={`Hide all areas in ${repo}`}
            onClick={onNone}
          >
            None
          </button>
        </div>
      )}
      {areas.length > 0 && (
        <ul className="areas__list">
          {areas.map((a) => {
            const on = !hidden.includes(a.key);
            const what = a.key === NO_AREA ? "titles without an area" : a.label;
            return (
              <li key={a.key}>
                <label
                  className={`areas__opt${a.key === NO_AREA ? " areas__opt--none" : ""}`}
                  title={`${a.label}: ${a.count} ${prsWord(a.count)}. ${on ? "Untick to hide" : "Tick to show"} ${what}.`}
                >
                  <input
                    type="checkbox"
                    className="areas__box"
                    checked={on}
                    aria-label={`${a.label}, ${a.count} ${prsWord(a.count)}`}
                    onChange={() => onToggle(a.key)}
                  />
                  <span className="areas__label">{a.label}</span>
                  <span className="areas__n" aria-hidden="true">
                    {a.count}
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      )}
      {allOff && <p className="areas__note">Every area is off, so none of this project’s pull requests are listed.</p>}
      {absent.length > 0 && (
        <p className="areas__note" title={absent.map(areaName).join(", ")}>
          Also off, with nothing listed now: {absent.map(areaName).join(", ")}
        </p>
      )}
    </div>
  );
}

function areaName(key: string): string {
  return key === NO_AREA ? NO_AREA_LABEL : key;
}
