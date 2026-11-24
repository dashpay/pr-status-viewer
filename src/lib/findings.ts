// What the row's status cell shows: two finding bubbles (blockers, then
// suggestions) or a short note, plus the words for the detail. The counts
// cover only the last review round of each reviewer (thepastaclaw and
// CodeRabbit). Each finding is agreed (settled with the reviewer's
// agreement), not agreed (closed without it), awaiting (resolved on GitHub
// after a push the reviewer has not re-reviewed yet) or open. Pure, so every
// combination is tested without rendering.

import type { FindingCounts, PullRequest, SeverityCount } from "../types";

export type Severity = keyof FindingCounts;
/** Severities that get a bubble on the row, in slot order. Nitpicks show in the detail only. */
export type BubbleSeverity = "blocking" | "suggestion";

/**
 * One severity's counts, made consistent: nothing negative, parts never above
 * the total. The parts are taken in order (agreed, then not agreed, then
 * awaiting), each capped by what is left, and `open` is the rest.
 */
export interface Tally {
  total: number;
  agreed: number;
  notAgreed: number;
  /** Resolved on GitHub, the fix not re-checked yet: its reviewer has not reviewed the current head. */
  awaiting: number;
  open: number;
  /** How many of `total` are CodeRabbit's. */
  coderabbit: number;
}

function whole(n: number): number {
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}

export function tally(c: SeverityCount): Tally {
  const total = whole(c.total);
  const agreed = Math.min(whole(c.agreed), total);
  const notAgreed = Math.min(whole(c.notAgreed), total - agreed);
  // `?? 0`: tolerate a snapshot from a backend that predates these fields.
  const awaiting = Math.min(whole(c.awaiting ?? 0), total - agreed - notAgreed);
  const coderabbit = Math.min(whole(c.coderabbit ?? 0), total);
  return { total, agreed, notAgreed, awaiting, open: total - agreed - notAgreed - awaiting, coderabbit };
}

const NOUN: Record<Severity, [string, string]> = {
  blocking: ["blocker", "blockers"],
  suggestion: ["suggestion", "suggestions"],
  nitpick: ["nitpick", "nitpicks"],
};

function noun(severity: Severity, n: number): string {
  return NOUN[severity][n === 1 ? 0 : 1];
}

/**
 * How much the bubble has to squeeze to stay inside its slot. `normal` fits
 * up to "🐰 👎3 👍87 / 92"; `tight` trims padding and gaps for larger numbers;
 * `squeeze` also narrows the digits and icons; `crush`, only ever needed with
 * all three counts shown and some of three digits, takes out the last air.
 */
export type BubbleFit = "normal" | "tight" | "squeeze" | "crush";

export interface Bubble {
  severity: BubbleSeverity;
  /** Thumbs-down count, shown first; null when no finding was closed without agreement. */
  notAgreed: number | null;
  /**
   * Hourglass count, shown between the thumbs; null when no finding is
   * awaiting re-review.
   */
  awaiting: number | null;
  /** Thumbs-up count; null when none was settled with agreement. */
  agreed: number | null;
  total: number;
  open: number;
  /**
   * Nothing open: every finding is agreed, not agreed or awaiting re-review.
   * The quiet form, because none of them is the author's move.
   */
  settled: boolean;
  /** Some of the total are CodeRabbit's findings: the bubble shows a rabbit. */
  coderabbit: boolean;
  /**
   * Accessible name and tooltip, e.g. "2 blockers in the last review round:
   * 1 agreed, 1 not agreed, 0 open. 1 from CodeRabbit."
   */
  label: string;
  fit: BubbleFit;
}

/**
 * A count as the bubble shows it: exact below 1000, then "1k" to "999k", so
 * no number takes more than three characters. The accessible name stays exact.
 */
export function countText(n: number): string {
  if (n < 1000) return String(n);
  return `${Math.min(999, Math.floor(n / 1000))}k`;
}

/**
 * Width class from what the bubble shows, in digits, counting each icon (a
 * thumb or the hourglass) as one and a half. "👎3 👍87 / 92" is 8 and fits as
 * is; "👎123 👍456 / 999", the widest with two icons, is 12 and squeezes; the
 * widest possible, "👎123 ⏳123 👍456 / 999", is 16.5 and is crushed. The
 * rabbit is not counted: the slot has room for it on top of the widest bubble
 * of each class (PrRow.css).
 */
function fitFor(notAgreed: number | null, awaiting: number | null, agreed: number | null, total: number): BubbleFit {
  const shown = [notAgreed, awaiting, agreed].filter((n): n is number => n != null);
  const width = [...shown, total].reduce((w, n) => w + countText(n).length, 0) + shown.length * 1.5;
  if (width <= 8) return "normal";
  if (width <= 10) return "tight";
  if (width <= 12.5) return "squeeze";
  return "crush";
}

/**
 * The words for one severity: "2 blockers in the last review round: 1 agreed,
 * 1 not agreed, 0 open. 1 from CodeRabbit." Findings awaiting re-review are
 * named only when there are some ("5 blockers in the last review round:
 * 0 agreed, 0 not agreed, 5 awaiting re-review, 0 open"), and CodeRabbit
 * only when it has findings in the total.
 */
export function tallyLabel(severity: Severity, t: Tally): string {
  const awaiting = t.awaiting > 0 ? `, ${t.awaiting} awaiting re-review` : "";
  const base =
    `${t.total} ${noun(severity, t.total)} in the last review round: ` +
    `${t.agreed} agreed, ${t.notAgreed} not agreed${awaiting}, ${t.open} open`;
  return t.coderabbit > 0 ? `${base}. ${t.coderabbit} from CodeRabbit.` : base;
}

/** The bubble for one severity, or null when it has no findings. */
export function bubble(severity: BubbleSeverity, counts: SeverityCount): Bubble | null {
  const t = tally(counts);
  if (t.total === 0) return null;
  const notAgreed = t.notAgreed > 0 ? t.notAgreed : null;
  const awaiting = t.awaiting > 0 ? t.awaiting : null;
  const agreed = t.agreed > 0 ? t.agreed : null;
  return {
    severity,
    notAgreed,
    awaiting,
    agreed,
    total: t.total,
    open: t.open,
    settled: t.open === 0,
    coderabbit: t.coderabbit > 0,
    label: tallyLabel(severity, t),
    fit: fitFor(notAgreed, awaiting, agreed, t.total),
  };
}

/** True when the bot has finished at least one review of the PR. */
export function hasReviewed(pr: PullRequest): boolean {
  const s = pr.bot.state;
  return pr.lastReviewAt != null || s === "approved" || s === "changesRequested" || s === "commented";
}

/** True when the bot has any finding on the PR, nitpicks included. */
export function hasFindings(pr: PullRequest): boolean {
  const f = pr.findingCounts;
  return tally(f.blocking).total + tally(f.suggestion).total + tally(f.nitpick).total > 0;
}

export type StatusCell =
  | { kind: "bubbles"; blocking: Bubble | null; suggestion: Bubble | null }
  | { kind: "note"; text: string; tone: "dim" | "draft" };

export const NO_FINDINGS = "No findings";
/** A draft the bot skipped, where nothing beside it says draft (the palette). */
export const DRAFT_NOT_REVIEWED = "Draft · not reviewed";
/** The same in a row's status cell, beside the row's own "Draft" tag. */
export const NOT_REVIEWED = "Not reviewed";
/** Why a draft has no review, for the status cell's tooltip. */
export const DRAFT_SKIPPED_NOTE = "PastaClaw skips drafts until they are marked ready for review";
export const NO_BOT_REVIEW = "No bot review";
export const FIRST_REVIEW = "First review";

/**
 * The status cell: the finding bubbles when there are blockers or
 * suggestions; otherwise a short note saying why there are none.
 */
export function statusCell(pr: PullRequest): StatusCell {
  const blocking = bubble("blocking", pr.findingCounts.blocking);
  const suggestion = bubble("suggestion", pr.findingCounts.suggestion);
  if (blocking || suggestion) return { kind: "bubbles", blocking, suggestion };
  if (hasReviewed(pr)) return { kind: "note", text: NO_FINDINGS, tone: "dim" };
  switch (pr.bot.state) {
    case "draft":
      return { kind: "note", text: NOT_REVIEWED, tone: "draft" };
    case "reviewing":
    case "queued":
      return { kind: "note", text: FIRST_REVIEW, tone: "dim" };
    case "other":
      return { kind: "note", text: pr.bot.title || "Unknown", tone: "dim" };
    default:
      return { kind: "note", text: NO_BOT_REVIEW, tone: "dim" };
  }
}

/**
 * One severity for the detail: "none", "0 of 3 settled", "2 of 2 settled
 * (1 not agreed)", "1 of 2 settled (1 not agreed), 1 from CodeRabbit",
 * "0 of 5 settled, 5 awaiting re-review". Settled means agreed or not agreed;
 * findings awaiting re-review are named after it when there are some.
 */
export function severityText(c: SeverityCount): string {
  const t = tally(c);
  if (t.total === 0) return "none";
  const settled = t.agreed + t.notAgreed;
  let text = `${settled} of ${t.total} settled`;
  if (t.notAgreed) text += ` (${t.notAgreed} not agreed)`;
  if (t.awaiting) text += `, ${t.awaiting} awaiting re-review`;
  if (t.coderabbit) text += `, ${t.coderabbit} from CodeRabbit`;
  return text;
}

/** What the detail's Findings fact adds after the counts: which rounds they cover. */
export const LAST_ROUND_NOTE = "(last review round of each reviewer)";

/**
 * "Blockers 1 of 2 settled (1 not agreed), 1 from CodeRabbit · Suggestions
 * 0 of 3 settled · Nitpicks none", over the last review round of each reviewer.
 */
export function findingCountsText(f: FindingCounts): string {
  return [
    `Blockers ${severityText(f.blocking)}`,
    `Suggestions ${severityText(f.suggestion)}`,
    `Nitpicks ${severityText(f.nitpick)}`,
  ].join(" · ");
}
