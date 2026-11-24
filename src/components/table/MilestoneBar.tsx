// The milestone switches: one toggle chip per milestone in the current view,
// on the filter line under the toolbar (FilterBar). Switched-off milestones' PRs are hidden
// from the list. Alt-click shows only that milestone.

import type { MouseEvent } from "react";
import { useApp } from "../../state/store";
import { NO_MILESTONE, soloToggles, type MilestoneChip } from "../../lib/milestones";
import "./MilestoneBar.css";

interface MilestoneBarProps {
  /** The milestones in the current view, before any filter. */
  chips: MilestoneChip[];
}

function chipTitle(chip: MilestoneChip, on: boolean): string {
  const what = chip.key === NO_MILESTONE ? "pull requests without a milestone" : chip.label;
  return `${on ? "Hide" : "Show"} ${what}. Alt-click to show only this one.`;
}

export function MilestoneBar({ chips }: MilestoneBarProps) {
  const hidden = useApp((s) => s.hiddenMilestones);
  const toggleMilestone = useApp((s) => s.toggleMilestone);
  const showAllMilestones = useApp((s) => s.showAllMilestones);
  const anyOff = chips.some((c) => hidden.includes(c.key));

  const onChip = (e: MouseEvent, chip: MilestoneChip) => {
    if (e.altKey) {
      for (const key of soloToggles(chips, hidden, chip.key)) toggleMilestone(key);
    } else {
      toggleMilestone(chip.key);
    }
  };

  return (
    <div className="ms-bar" role="group" aria-labelledby="ms-bar-label">
      <span className="ms-bar__label dim" id="ms-bar-label">
        Milestones
      </span>
      {chips.map((chip) => {
        const on = !hidden.includes(chip.key);
        return (
          <button
            key={chip.key}
            type="button"
            className={`ms-chip${chip.key === NO_MILESTONE ? " ms-chip--none" : ""}`}
            aria-pressed={on}
            aria-label={`${chip.label}, ${chip.count} pull ${chip.count === 1 ? "request" : "requests"}`}
            title={chipTitle(chip, on)}
            onClick={(e) => onChip(e, chip)}
          >
            <span className="ms-chip__box" aria-hidden="true">
              {on ? "✓" : ""}
            </span>
            <span className="ms-chip__label">{chip.label}</span>
            <span className="ms-chip__count">{chip.count}</span>
          </button>
        );
      })}
      {anyOff && (
        <button type="button" className="btn ghost ms-bar__all" onClick={showAllMilestones}>
          Show all
        </button>
      )}
    </div>
  );
}
