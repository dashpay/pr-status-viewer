// The filter line under the toolbar: the drafts switch, the created recently
// switch, then the milestone switches. Each part shows only when it has
// something to offer, and the line shows when any does, so the two switches
// never depend on the milestones having a choice.

import type { ReactNode } from "react";
import { useApp } from "../../state/store";
import "./MilestoneBar.css";
import "./FilterBar.css";

export function FilterBar({ children }: { children: ReactNode }) {
  return <div className="filter-bar">{children}</div>;
}

interface DraftsSwitchProps {
  /** Drafts in the current view, before any filter. */
  count: number;
}

/**
 * One toggle that shows or hides every draft. Looks like a milestone chip
 * (ticked and filled when drafts are shown, empty and dashed when hidden)
 * but stands apart from them: it is not a milestone.
 */
export function DraftsSwitch({ count }: DraftsSwitchProps) {
  const showDrafts = useApp((s) => s.showDrafts);
  const setShowDrafts = useApp((s) => s.setShowDrafts);
  return (
    <button
      type="button"
      className="ms-chip filter-bar__drafts"
      aria-pressed={showDrafts}
      aria-label={`Drafts, ${count} pull ${count === 1 ? "request" : "requests"}`}
      title={showDrafts ? "Hide draft pull requests" : "Show draft pull requests"}
      onClick={() => setShowDrafts(!showDrafts)}
    >
      <span className="ms-chip__box" aria-hidden="true">
        {showDrafts ? "✓" : ""}
      </span>
      <span className="ms-chip__label">Drafts</span>
      <span className="ms-chip__count">{count}</span>
    </button>
  );
}

interface RecentSwitchProps {
  /** Pull requests in the current view created in the last 3 days, before any filter. */
  count: number;
}

/**
 * One toggle that, when ticked, lists only pull requests created in the last
 * 3 days. Unlike the drafts switch it narrows the list when ticked; unticked
 * (the default) it hides nothing.
 */
export function RecentSwitch({ count }: RecentSwitchProps) {
  const recentOnly = useApp((s) => s.recentOnly);
  const setRecentOnly = useApp((s) => s.setRecentOnly);
  return (
    <button
      type="button"
      className="ms-chip filter-bar__recent"
      aria-pressed={recentOnly}
      aria-label={`Created recently, ${count} pull ${count === 1 ? "request" : "requests"}`}
      title={recentOnly ? "Show pull requests of any age" : "Show only pull requests created in the last 3 days"}
      onClick={() => setRecentOnly(!recentOnly)}
    >
      <span className="ms-chip__box" aria-hidden="true">
        {recentOnly ? "✓" : ""}
      </span>
      <span className="ms-chip__label">Created recently</span>
      <span className="ms-chip__count">{count}</span>
    </button>
  );
}
