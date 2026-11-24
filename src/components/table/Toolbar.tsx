// Filter field, Group and Sort controls, and the "By subcrate" box that
// splits each project group by subcrate. Sits above the list and does not scroll.

import type { RefObject } from "react";
import { useApp, type GroupBy, type SortBy } from "../../state/store";

interface ToolbarProps {
  inputRef: RefObject<HTMLInputElement | null>;
  /** Esc in the filter: clear it and return to the list. */
  onEscape(): void;
}

const GROUPS: { value: GroupBy; label: string }[] = [
  { value: "project", label: "Project" },
  { value: "status", label: "Status" },
];

const SORTS: { value: SortBy; label: string }[] = [
  { value: "finishesFirst", label: "Finishes first" },
  { value: "updated", label: "Updated" },
];

/** Tooltip of the subcrate box while grouping by status, when it has no effect. */
export const SUBCRATE_DISABLED_HINT = "Group by project to split by subcrate";

export function Toolbar({ inputRef, onEscape }: ToolbarProps) {
  const query = useApp((s) => s.query);
  const setQuery = useApp((s) => s.setQuery);
  const groupBy = useApp((s) => s.groupBy);
  const setGroupBy = useApp((s) => s.setGroupBy);
  const sortBy = useApp((s) => s.sortBy);
  const setSortBy = useApp((s) => s.setSortBy);
  const bySubcrate = useApp((s) => s.bySubcrate);
  const setBySubcrate = useApp((s) => s.setBySubcrate);
  const subcrateOff = groupBy !== "project";
  const subcrateHint = subcrateOff
    ? SUBCRATE_DISABLED_HINT
    : bySubcrate
      ? "Show each project's pull requests in one list"
      : "Split each project into one list per subcrate (the scope in the title)";

  return (
    <div className="pr-toolbar">
      <div className="pr-toolbar__filter">
        <input
          ref={inputRef}
          className="input"
          type="search"
          placeholder="Filter pull requests"
          aria-label="Filter pull requests"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              onEscape();
            }
          }}
          spellCheck={false}
          autoComplete="off"
        />
        {!query && (
          <span className="kbd pr-toolbar__hint" aria-hidden="true">
            /
          </span>
        )}
      </div>
      <span className="pr-toolbar__spacer" />
      <span className="dim" id="pr-toolbar-group">
        Group
      </span>
      <div className="seg" role="group" aria-labelledby="pr-toolbar-group">
        {GROUPS.map((g) => (
          <button key={g.value} type="button" aria-pressed={groupBy === g.value} onClick={() => setGroupBy(g.value)}>
            {g.label}
          </button>
        ))}
      </div>
      <span className="dim" id="pr-toolbar-sort">
        Sort
      </span>
      <div className="seg" role="group" aria-labelledby="pr-toolbar-sort">
        {SORTS.map((s) => (
          <button key={s.value} type="button" aria-pressed={sortBy === s.value} onClick={() => setSortBy(s.value)}>
            {s.label}
          </button>
        ))}
      </div>
      {/* The label keeps its tooltip while the box is disabled (a disabled
          control shows none of its own). */}
      <label className={subcrateOff ? "pr-toolbar__box is-disabled" : "pr-toolbar__box"} title={subcrateHint}>
        <input
          type="checkbox"
          checked={bySubcrate}
          disabled={subcrateOff}
          aria-describedby={subcrateOff ? "pr-toolbar-subcrate-hint" : undefined}
          onChange={(e) => setBySubcrate(e.target.checked)}
        />
        By subcrate
      </label>
      {subcrateOff && (
        <span id="pr-toolbar-subcrate-hint" hidden>
          {SUBCRATE_DISABLED_HINT}
        </span>
      )}
    </div>
  );
}
