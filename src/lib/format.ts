// Pure text and time helpers for the PR table and palette. Every function
// that depends on the clock takes `now` (milliseconds) as an argument, so the
// UI decides when to re-render and tests can pin the time.

import type { BotReview, HumanReview, HumanReviewState, PullRequest } from "../types";
import { DRAFT_NOT_REVIEWED, NO_BOT_REVIEW, statusCell } from "./findings";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Below this a running review reads "finishing up" rather than a minute count. */
export const FINISHING_UP_MS = 3 * MINUTE;
/** Below this a visible countdown wants a one-second tick. */
export const FAST_TICK_MS = 2 * MINUTE;

export const QUEUED_ESTIMATE_NOTE = "Estimated from the bot's median review time";

/** Parses an RFC 3339 string; null for null or garbage. */
export function parseTime(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
}

/** "just now", "10 min ago", "2 h ago", "3 d ago". Future times read "just now". */
export function relativeAgo(iso: string | null | undefined, now: number): string | null {
  const t = parseTime(iso);
  if (t == null) return null;
  const ago = now - t;
  if (ago < MINUTE) return "just now";
  if (ago < HOUR) return `${Math.floor(ago / MINUTE)} min ago`;
  if (ago < DAY) return `${Math.floor(ago / HOUR)} h ago`;
  return `${Math.floor(ago / DAY)} d ago`;
}

/** An approximate span for estimates: "~20 min", "~1 h 5 min", "~1 min". */
export function approxSpan(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / MINUTE));
  if (minutes < 60) return `~${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `~${h} h ${m} min` : `~${h} h`;
}

/** An exact elapsed span: "16 s", "3 min 47 s", "1 h 5 min", "2 d 3 h". */
export function exactSpan(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s} s`;
  if (s < 3600) {
    const rest = s % 60;
    return rest ? `${Math.floor(s / 60)} min ${rest} s` : `${Math.floor(s / 60)} min`;
  }
  if (s < 86400) {
    const m = Math.floor((s % 3600) / 60);
    return m ? `${Math.floor(s / 3600)} h ${m} min` : `${Math.floor(s / 3600)} h`;
  }
  const h = Math.floor((s % 86400) / 3600);
  return h ? `${Math.floor(s / 86400)} d ${h} h` : `${Math.floor(s / 86400)} d`;
}

/** The exact local date and time, for tooltips. */
export function exactLocal(iso: string | null | undefined): string | null {
  const t = parseTime(iso);
  if (t == null) return null;
  return new Date(t).toLocaleString(undefined, {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Local clock time only, e.g. "14:32". */
export function clockTime(iso: string | null | undefined): string | null {
  const t = parseTime(iso);
  if (t == null) return null;
  return new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/** What the time column shows for a PR. */
export interface TimeCell {
  text: string;
  /** Bold for a running review's countdown. */
  strong: boolean;
  /** Dim for past times and placeholders. */
  dim: boolean;
  /** Tooltip: exact local time, plus a note for estimates. */
  title: string | null;
  /** While a review is running or queued: what it is doing, in words, for a second line. */
  live: { text: string; tone: "run" | "wait" } | null;
}

/** "Reviewing · Phase 1" or "Queued · #3"; null when no review is running or queued. */
export function liveText(bot: BotReview): string | null {
  if (bot.state === "reviewing") return bot.phaseLabel ? `Reviewing · ${bot.phaseLabel}` : "Reviewing";
  if (bot.state === "queued") return bot.position != null ? `Queued · #${bot.position}` : "Queued";
  return null;
}

/**
 * The time column: the countdown while a review is running or queued (with
 * what it is doing underneath), otherwise when the last review came in.
 */
export function timeCell(pr: PullRequest, now: number): TimeCell {
  const bot = pr.bot;
  switch (bot.state) {
    case "reviewing": {
      const live = { text: liveText(bot) ?? "Reviewing", tone: "run" as const };
      const end = parseTime(bot.finishesAt);
      const title = end == null ? null : `Expected to finish ${exactLocal(bot.finishesAt)}`;
      if (end == null) {
        if (bot.overdue) return { text: "running late", strong: true, dim: false, title: "Taking longer than usual", live };
        return { text: "no estimate", strong: false, dim: true, title: null, live };
      }
      const left = end - now;
      // "longer than usual" (the bot's wording) does not fit the time column.
      if (bot.overdue || left <= 0) {
        return { text: "running late", strong: true, dim: false, title: `Taking longer than usual. ${title ?? ""}`.trim(), live };
      }
      if (left < FINISHING_UP_MS) return { text: "finishing up", strong: true, dim: false, title, live };
      return { text: approxSpan(left), strong: true, dim: false, title, live };
    }
    case "queued": {
      const live = { text: liveText(bot) ?? "Queued", tone: "wait" as const };
      const end = parseTime(bot.finishesAt);
      if (end == null || end <= now) return { text: "no estimate", strong: false, dim: true, title: null, live };
      return {
        text: approxSpan(end - now),
        strong: false,
        dim: false,
        title: `${QUEUED_ESTIMATE_NOTE}: about ${exactLocal(bot.finishesAt)}`,
        live,
      };
    }
    default: {
      const ago = relativeAgo(pr.lastReviewAt, now);
      if (ago == null) return { text: "—", strong: false, dim: true, title: null, live: null };
      return { text: ago, strong: false, dim: true, title: `Last review ${exactLocal(pr.lastReviewAt)}`, live: null };
    }
  }
}

/**
 * Milliseconds until a visible countdown next changes in a way that matters,
 * or null when the PR has no countdown. Used to pick the tick rate.
 */
export function countdownLeft(bot: BotReview, now: number): number | null {
  if (bot.state !== "reviewing" && bot.state !== "queued") return null;
  const end = parseTime(bot.finishesAt);
  return end == null ? null : end - now;
}

/** True when this PR's countdown is close enough to need a one-second tick. */
export function needsFastTick(bot: BotReview, now: number): boolean {
  const left = countdownLeft(bot, now);
  return left != null && left > -MINUTE && left < FAST_TICK_MS;
}

/** Plain label for a bot state, for the palette. */
export function botStatusText(bot: BotReview): string {
  switch (bot.state) {
    case "reviewing":
    case "queued":
      return liveText(bot) ?? "Reviewing";
    case "approved":
      return "Approved";
    case "changesRequested":
      return "Changes requested";
    case "commented":
      return "Comments only";
    case "draft":
      return DRAFT_NOT_REVIEWED;
    case "other":
      return bot.title || "Unknown";
    case "none":
      return NO_BOT_REVIEW;
  }
}

const HUMAN_STATE_WORDS: Record<HumanReviewState, string> = {
  approved: "approved",
  changesRequested: "requested changes",
  commented: "commented",
  dismissed: "dismissed",
  requested: "requested",
};

export function humanStateText(h: HumanReview): string {
  // "lklimek requested" would read as if lklimek asked for something.
  if (h.state === "requested") return `review requested from ${h.login}`;
  return `${h.login} ${HUMAN_STATE_WORDS[h.state]}`;
}

function names(list: HumanReview[]): string {
  const [first, ...rest] = list.map((h) => h.login);
  return rest.length ? `${first} and ${rest.length} more` : first;
}

/** The single most important fact about human review, or null. */
export function humanSummary(pr: PullRequest): string | null {
  const by = (s: HumanReviewState) => pr.humans.filter((h) => h.state === s);
  const changes = by("changesRequested");
  if (changes.length) return `changes requested by ${names(changes)}`;
  const approved = by("approved");
  if (pr.reviewDecision === "reviewRequired" && approved.length === 0) return "needs 1 human review";
  if (approved.length) return `approved by ${names(approved)}`;
  const requested = by("requested");
  if (requested.length) return `review requested from ${names(requested)}`;
  return null;
}

/** "1 blocking · 2 suggestions · 1 nitpick", or "none" when all are zero. */
export function findingsText(f: { blocking: number; suggestion: number; nitpick: number }): string {
  const parts: string[] = [];
  if (f.blocking) parts.push(`${f.blocking} blocking`);
  if (f.suggestion) parts.push(`${f.suggestion} ${f.suggestion === 1 ? "suggestion" : "suggestions"}`);
  if (f.nitpick) parts.push(`${f.nitpick} ${f.nitpick === 1 ? "nitpick" : "nitpicks"}`);
  return parts.length ? parts.join(" · ") : "none";
}

export interface SubLineOptions {
  /** Show the repo (when rows are not grouped by project). */
  showRepo: boolean;
}

/** The parts of a row's second line, in order. The first is the PR number. */
export function subLineParts(pr: PullRequest, now: number, opts: SubLineOptions): string[] {
  const parts: string[] = [];
  if (opts.showRepo) parts.push(pr.repo);
  parts.push(`#${pr.number}`);
  parts.push(`+${pr.additions} −${pr.deletions}`);
  if (pr.milestone) parts.push(pr.milestone);
  if (!pr.isMine) parts.push(pr.author);
  // The review bot's posts so far; none yet says nothing (the status cell covers it).
  if (pr.rounds != null && pr.rounds > 0) parts.push(`round ${pr.rounds}`);
  const bot = pr.bot;
  // The status cell shows an unrecognised state's title only when there are no
  // findings to show, and is too narrow for a sentence such as "The review
  // ended without a verdict".
  if (bot.state === "other" && bot.title) {
    const cell = statusCell(pr);
    if (cell.kind !== "note" || cell.text !== bot.title || bot.title.length > 18) parts.push(bot.title);
  }
  if ((bot.state === "queued" || bot.state === "reviewing") && bot.priority) parts.push("priority");
  const human = humanSummary(pr);
  if (human) parts.push(human);
  if (bot.state === "queued") {
    const start = parseTime(bot.startsAt);
    if (start != null && start > now) parts.push(`starts in ${approxSpan(start - now)}`);
  }
  const next = nextStepRowLabel(pr);
  if (next) parts.push(next);
  return parts;
}

/**
 * The next step's label for a row's second line, or null when there is none
 * or the row already says the same thing ("Draft" on a draft, whose title
 * line starts with a Draft tag; "Waiting for bots" while the bot is
 * reviewing or queued), or says something better ("Your move" while an
 * agent works on the PR in T3 Code: the title's "T3 working" tag says who
 * has it).
 */
export function nextStepRowLabel(pr: PullRequest): string | null {
  const step = pr.nextStep;
  if (!step) return null;
  if (step.stage === "draft" && pr.isDraft) return null;
  if (step.stage === "waiting-bots" && (pr.bot.state === "reviewing" || pr.bot.state === "queued")) return null;
  // The same test as merge.ts `t3Working`, which this file cannot import (merge.ts imports it).
  if (step.stage === "waiting-self-review" && pr.isMine && pr.t3Threads.some((t) => t.state === "working")) {
    return null;
  }
  return step.label;
}

/** Bot verdict for the Reviewers line of the detail. */
export function botReviewerText(bot: BotReview): string {
  switch (bot.state) {
    case "reviewing":
      return "PastaClaw reviewing";
    case "queued":
      return "PastaClaw queued";
    case "approved":
      return "PastaClaw approved";
    case "changesRequested":
      return "PastaClaw requested changes";
    case "commented":
      return "PastaClaw commented";
    case "draft":
      return "PastaClaw skipped the draft";
    case "other":
      return `PastaClaw: ${bot.title}`;
    case "none":
      return "No bot review";
  }
}

/** Case-insensitive match of every word in `query` against `text`. */
export function matchesWords(text: string, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const hay = text.toLowerCase();
  return q.split(/\s+/).every((w) => hay.includes(w));
}
