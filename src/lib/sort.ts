// Row order within a group.

import type { PullRequest } from "../types";
import type { SortBy } from "../state/store";
import { parseTime } from "./format";

/** When the bot's review is expected to end, for reviewing and queued PRs. */
export function finishTime(pr: PullRequest): number | null {
  const bot = pr.bot;
  if (bot.state !== "reviewing" && bot.state !== "queued") return null;
  return parseTime(bot.finishesAt);
}

/** When the bot's last review came in, for PRs it is not reviewing or queued for now. */
export function lastReviewTime(pr: PullRequest): number | null {
  if (pr.bot.state === "reviewing" || pr.bot.state === "queued") return null;
  return parseTime(pr.lastReviewAt);
}

function byUpdatedDesc(a: PullRequest, b: PullRequest): number {
  return (parseTime(b.updatedAt) ?? 0) - (parseTime(a.updatedAt) ?? 0) || a.id.localeCompare(b.id);
}

/**
 * "Finishes first": PRs with a finish time, soonest first; then PRs the bot
 * has reviewed, newest review first (the time their row shows); then the rest
 * by most recently updated.
 */
function byFinishesFirst(a: PullRequest, b: PullRequest): number {
  const fa = finishTime(a);
  const fb = finishTime(b);
  if (fa != null && fb != null) return fa - fb || byUpdatedDesc(a, b);
  if (fa != null) return -1;
  if (fb != null) return 1;
  const va = lastReviewTime(a);
  const vb = lastReviewTime(b);
  if (va != null && vb != null) return vb - va || byUpdatedDesc(a, b);
  if (va != null) return -1;
  if (vb != null) return 1;
  return byUpdatedDesc(a, b);
}

/** A sorted copy. */
export function sortPrs(prs: readonly PullRequest[], sortBy: SortBy): PullRequest[] {
  return [...prs].sort(sortBy === "finishesFirst" ? byFinishesFirst : byUpdatedDesc);
}
