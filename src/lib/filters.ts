// The list's switch filters, drafts then created recently then milestone
// then area, applied together so the list can say truthfully what each one
// hides.

import type { PullRequest } from "../types";
import { areaVisible } from "./areas";
import { milestoneShown } from "./milestones";

export interface SwitchFiltered {
  /** The pull requests every switch lets through, in input order. */
  visible: PullRequest[];
  /** Drafts, hidden by the drafts switch, whatever their milestone or area. */
  byDraft: PullRequest[];
  /** Older than `RECENT_MS`, hidden by the created recently switch, and not a hidden draft. */
  byAge: PullRequest[];
  /** Hidden by milestone and not hidden by the two switches before it, whether or not their area is also off. */
  byMilestone: PullRequest[];
  /** Hidden by area only: not hidden by the switches before it, and their milestone is on. */
  byArea: PullRequest[];
}

/** How recent "created recently" is: 3 days. */
export const RECENT_MS = 72 * 60 * 60 * 1000;

/**
 * True when the PR was created in the 3 days before `now` (ms). A PR whose
 * creation time cannot be read counts as not recent.
 */
export function createdRecently(pr: PullRequest, now: number): boolean {
  const created = Date.parse(pr.createdAt);
  if (Number.isNaN(created)) return false;
  return now - created <= RECENT_MS;
}

/** True when the created recently switch lets the PR through. */
export function recentShown(pr: PullRequest, recentOnly: boolean, now: number): boolean {
  return !recentOnly || createdRecently(pr, now);
}

/** True when the drafts switch lets the PR through. */
export function draftShown(pr: PullRequest, showDrafts: boolean): boolean {
  return showDrafts || !pr.isDraft;
}

/**
 * Splits `prs` by the drafts, created recently, milestone and area switches.
 * A pull request hidden for several reasons counts once, under the first of
 * drafts, created recently, milestone, area. `now` (ms) only matters while
 * `recentOnly` is on.
 */
export function applySwitches(
  prs: readonly PullRequest[],
  hiddenMilestones: readonly string[],
  hiddenAreas: Readonly<Record<string, readonly string[]>>,
  showDrafts = true,
  recentOnly = false,
  now = 0,
): SwitchFiltered {
  const out: SwitchFiltered = { visible: [], byDraft: [], byAge: [], byMilestone: [], byArea: [] };
  for (const pr of prs) {
    if (!draftShown(pr, showDrafts)) out.byDraft.push(pr);
    else if (!recentShown(pr, recentOnly, now)) out.byAge.push(pr);
    else if (!milestoneShown(pr, hiddenMilestones)) out.byMilestone.push(pr);
    else if (!areaVisible(pr, hiddenAreas)) out.byArea.push(pr);
    else out.visible.push(pr);
  }
  return out;
}

/** How many pull requests each switch hides, each counted once (see `applySwitches`). */
export interface HiddenCounts {
  byDraft: number;
  /** Hidden by the created recently switch. Omitted: none. */
  byAge?: number;
  byMilestone: number;
  byArea: number;
}

/**
 * "3 drafts hidden · 5 older than 3 days hidden · 4 hidden by milestone ·
 * 2 hidden by area", each part only when non-zero; null when nothing is hidden.
 */
export function hiddenNote({ byDraft, byAge = 0, byMilestone, byArea }: HiddenCounts): string | null {
  const parts = [
    byDraft > 0 ? `${byDraft} ${byDraft === 1 ? "draft" : "drafts"} hidden` : null,
    byAge > 0 ? `${byAge} older than 3 days hidden` : null,
    byMilestone > 0 ? `${byMilestone} hidden by milestone` : null,
    byArea > 0 ? `${byArea} hidden by area` : null,
  ].filter((p): p is string => p != null);
  return parts.length ? parts.join(" · ") : null;
}

/** How many of `prs` are drafts. */
export function draftCount(prs: readonly PullRequest[]): number {
  return prs.reduce((n, pr) => n + (pr.isDraft ? 1 : 0), 0);
}

/**
 * Whether the drafts switch is worth showing: the view has a draft, or drafts
 * are hidden (so they can always be switched back on).
 */
export function showDraftsSwitch(drafts: number, showDrafts: boolean): boolean {
  return drafts > 0 || !showDrafts;
}

/** How many of `prs` were created in the 3 days before `now` (ms). */
export function recentCount(prs: readonly PullRequest[], now: number): number {
  return prs.reduce((n, pr) => n + (createdRecently(pr, now) ? 1 : 0), 0);
}

/**
 * Whether the created recently switch is worth showing: the view has a
 * recent pull request, or the switch is on (so it can always be turned off).
 */
export function showRecentSwitch(recent: number, recentOnly: boolean): boolean {
  return recent > 0 || recentOnly;
}

/** The repos of `prs`, once each, in first-seen order. */
export function reposOf(prs: readonly PullRequest[]): string[] {
  return [...new Set(prs.map((pr) => pr.repo))];
}
