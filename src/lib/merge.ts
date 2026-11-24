// Text helpers for the squash-merge button, its confirmation and the T3 Code
// indicator. Pure, like format.ts: anything that depends on the clock takes `now`.

import type { BotReview, PullRequest, T3Thread } from "../types";
import { relativeAgo } from "./format";

/**
 * The reason the backend gives when the viewer cannot merge in the repo at
 * all. The merge button is hidden for it rather than shown disabled.
 */
export const NO_PERMISSION_REASON = "You do not have permission to merge in this repository.";

/** True when the merge button belongs on this PR at all. */
export function showMergeButton(pr: PullRequest): boolean {
  return pr.merge.ready || pr.merge.reason !== NO_PERMISSION_REASON;
}

/** "All resolved", "1 unresolved", "3 unresolved". */
export function commentsText(unresolved: number): string {
  return unresolved <= 0 ? "All resolved" : `${unresolved} unresolved`;
}

function botWarning(bot: BotReview): string | null {
  switch (bot.state) {
    case "approved":
      return null;
    case "reviewing":
    case "queued":
      return "PastaClaw has not finished reviewing.";
    case "changesRequested":
      return "PastaClaw requested changes.";
    case "commented":
      return "PastaClaw commented but has not approved.";
    case "other":
      return `PastaClaw has not approved (${bot.title || "unknown state"}).`;
    case "draft":
    case "none":
      return "PastaClaw has not reviewed this pull request.";
  }
}

/**
 * Plain one-line warnings for the merge confirmation. They inform; they do
 * not block the merge.
 */
export function mergeWarnings(pr: PullRequest): string[] {
  const out: string[] = [];
  if (pr.ci === "failing") out.push("CI is failing.");
  else if (pr.ci === "pending") out.push("CI is still running.");
  const bot = botWarning(pr.bot);
  if (bot) out.push(bot);
  const changes = pr.humans.filter((h) => h.state === "changesRequested").map((h) => h.login);
  if (changes.length === 1) out.push(`${changes[0]} requested changes.`);
  else if (changes.length > 1) out.push(`${changes.join(", ")} requested changes.`);
  else if (pr.reviewDecision === "changesRequested") out.push("A reviewer requested changes.");
  return out;
}

/** True when an agent is running in any of the PR's T3 Code threads. */
export function t3Working(pr: PullRequest): boolean {
  return pr.t3Threads.some((t) => t.state === "working");
}

/**
 * The tag beside a row's title: "T3 working" while any thread runs, else
 * "T3 waiting" while any thread waits on something it started, else none.
 */
export function t3RowTag(pr: PullRequest): { state: "working" | "waiting"; label: string; title: string } | null {
  if (t3Working(pr)) {
    return { state: "working", label: "T3 working", title: "An agent is working on this pull request in T3 Code" };
  }
  const waiting = pr.t3Threads.filter((t) => t.state === "waiting");
  if (waiting.length === 0) return null;
  return {
    state: "waiting",
    label: "T3 waiting",
    title: waiting.some(isWatching)
      ? "A T3 Code thread is watching this pull request and resumes when it changes"
      : "A T3 Code thread is waiting on something it started and resumes on its own",
  };
}

/** A snapshot from an older backend may lack `watching`; read that as false. */
function isWatching(thread: T3Thread): boolean {
  return thread.watching === true;
}

const T3_ORDER: Record<T3Thread["state"], number> = { working: 0, waiting: 1, idle: 2 };

/** Working threads first, then waiting, then idle; otherwise the order the backend gave. */
export function sortT3Threads(threads: T3Thread[]): T3Thread[] {
  return [...threads].sort((a, b) => (T3_ORDER[a.state] ?? 2) - (T3_ORDER[b.state] ?? 2));
}

/**
 * "working now", "waiting, last active 5 min ago · watching this PR",
 * "idle, last active 2 h ago", or "idle" when the time is unknown.
 */
export function t3StateText(thread: T3Thread, now: number): string {
  if (thread.state === "working") return "working now";
  const word = thread.state === "waiting" ? "waiting" : "idle";
  const ago = relativeAgo(thread.lastActiveAt, now);
  const base = ago ? `${word}, last active ${ago}` : word;
  return isWatching(thread) ? `${base} · watching this PR` : base;
}
