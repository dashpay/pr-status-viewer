// Milestones present among a list of PRs, and the show/hide filter over them.
//
// A milestone is keyed by its title exactly as GitHub shows it; PRs without
// one use `NO_MILESTONE` (an empty string, which no real title can be).

import type { PullRequest } from "../types";
import { NO_MILESTONE } from "../state/store";

export { NO_MILESTONE };

export interface MilestoneChip {
  /** The title as GitHub has it, or `NO_MILESTONE`. */
  key: string;
  /** Display text: the title, or "No milestone". */
  label: string;
  /** PRs in the list with this milestone. */
  count: number;
}

export const NO_MILESTONE_LABEL = "No milestone";

/** The key a PR is filed under. */
export function milestoneKey(pr: PullRequest): string {
  return pr.milestone ?? NO_MILESTONE;
}

/**
 * Version order: digit runs compare as numbers and a leading "v" is ignored,
 * so "v5.2.0" < "v5.10.0" < "v6.0.0" and "24.1" < "25". Ties fall back to the
 * raw title so the order is stable.
 */
export function compareMilestones(a: string, b: string): number {
  const strip = (s: string) => s.replace(/^v(?=\d)/i, "");
  const byVersion = strip(a).localeCompare(strip(b), "en", { numeric: true, sensitivity: "base" });
  if (byVersion !== 0) return byVersion;
  return a < b ? -1 : a > b ? 1 : 0;
}

/** The milestones among `prs` with their counts, in version order, "No milestone" last. */
export function milestoneChips(prs: readonly PullRequest[]): MilestoneChip[] {
  const counts = new Map<string, number>();
  for (const pr of prs) {
    const key = milestoneKey(pr);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const titled = [...counts.keys()].filter((k) => k !== NO_MILESTONE).sort(compareMilestones);
  const chips: MilestoneChip[] = titled.map((key) => ({ key, label: key, count: counts.get(key) ?? 0 }));
  const none = counts.get(NO_MILESTONE);
  if (none) chips.push({ key: NO_MILESTONE, label: NO_MILESTONE_LABEL, count: none });
  return chips;
}

/**
 * Whether the chip row is worth showing: there is a choice to make (two or
 * more chips, at least one a real milestone). A chip in view that is switched
 * off always keeps the row, so it can be switched back on.
 */
export function showMilestoneRow(chips: readonly MilestoneChip[], hidden: readonly string[]): boolean {
  if (chips.some((c) => hidden.includes(c.key))) return true;
  return chips.length > 1 && chips.some((c) => c.key !== NO_MILESTONE);
}

/**
 * The keys to toggle so that only `key` is shown among `chips`. When `key` is
 * already the only one shown, the keys that bring the others back instead.
 * Keys outside `chips` (switched off in another view) are never touched.
 */
export function soloToggles(chips: readonly MilestoneChip[], hidden: readonly string[], key: string): string[] {
  const others = chips.filter((c) => c.key !== key).map((c) => c.key);
  const alreadySolo = !hidden.includes(key) && others.every((k) => hidden.includes(k));
  if (alreadySolo) return others;
  return [...(hidden.includes(key) ? [key] : []), ...others.filter((k) => !hidden.includes(k))];
}

/** True when the PR's milestone is not switched off. */
export function milestoneShown(pr: PullRequest, hidden: readonly string[]): boolean {
  return hidden.length === 0 || !hidden.includes(milestoneKey(pr));
}

/** The PRs whose milestone is switched on. */
export function filterByMilestone(prs: readonly PullRequest[], hidden: readonly string[]): PullRequest[] {
  return hidden.length === 0 ? [...prs] : prs.filter((pr) => milestoneShown(pr, hidden));
}
