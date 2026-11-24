import { describe, expect, it } from "vitest";
import type { BotReview, BotReviewing, PullRequest, T3Thread } from "../types";
import { mockSnapshot } from "../mock/snapshot";
import {
  approxSpan,
  botStatusText,
  exactSpan,
  findingsText,
  humanSummary,
  needsFastTick,
  nextStepRowLabel,
  relativeAgo,
  subLineParts,
  timeCell,
} from "./format";

const NOW = Date.parse("2026-11-22T12:00:00Z");
const iso = (offsetSec: number) => new Date(NOW + offsetSec * 1000).toISOString();

function reviewing(patch: Partial<BotReviewing> = {}): BotReviewing {
  return {
    state: "reviewing",
    phase: "phase1",
    phaseLabel: "Phase 1",
    progress: 0.2,
    finishesAt: iso(1200),
    startedAt: iso(-200),
    overdue: false,
    tier: "normal",
    priority: false,
    waitedSeconds: 16,
    steps: [],
    findings: null,
    ...patch,
  };
}

function basePr(patch: Partial<PullRequest> = {}): PullRequest {
  const pr = mockSnapshot(NOW).projects[0].prs[0];
  return {
    ...pr,
    milestone: null,
    rounds: null,
    humans: [],
    reviewDecision: null,
    nextStep: null,
    lastReviewAt: null,
    findingCounts: ZERO_COUNTS,
    ...patch,
  };
}

const ZERO_COUNTS: PullRequest["findingCounts"] = {
  blocking: { total: 0, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 },
  suggestion: { total: 0, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 },
  nitpick: { total: 0, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 },
};

/** The time cell of a PR with this bot state. */
function cell(bot: BotReview, patch: Partial<PullRequest> = {}) {
  return timeCell(basePr({ bot, ...patch }), NOW);
}

function step(stage: string, label: string): PullRequest["nextStep"] {
  return { stage, label, actor: "unknown", detail: "A sentence that never shows on the row." };
}

describe("relativeAgo", () => {
  it("reads minutes, hours and days, rounding down", () => {
    expect(relativeAgo(iso(-20), NOW)).toBe("just now");
    expect(relativeAgo(iso(-620), NOW)).toBe("10 min ago");
    expect(relativeAgo(iso(-3180), NOW)).toBe("53 min ago");
    expect(relativeAgo(iso(-9180), NOW)).toBe("2 h ago");
    expect(relativeAgo(iso(-259200), NOW)).toBe("3 d ago");
  });
  it("is null for a missing or broken time", () => {
    expect(relativeAgo(null, NOW)).toBeNull();
    expect(relativeAgo("not a date", NOW)).toBeNull();
  });
});

describe("spans", () => {
  it("approximates to whole minutes, at least one", () => {
    expect(approxSpan(1196_000)).toBe("~20 min");
    expect(approxSpan(2815_000)).toBe("~47 min");
    expect(approxSpan(10_000)).toBe("~1 min");
    expect(approxSpan(65 * 60_000)).toBe("~1 h 5 min");
    expect(approxSpan(120 * 60_000)).toBe("~2 h");
  });
  it("gives exact elapsed spans", () => {
    expect(exactSpan(16)).toBe("16 s");
    expect(exactSpan(227)).toBe("3 min 47 s");
    expect(exactSpan(180)).toBe("3 min");
    expect(exactSpan(3900)).toBe("1 h 5 min");
    expect(exactSpan(-5)).toBe("0 s");
  });
});

describe("timeCell", () => {
  it("counts a running review down in bold, with the phase in words underneath", () => {
    expect(cell(reviewing())).toMatchObject({
      text: "~20 min",
      strong: true,
      live: { text: "Reviewing · Phase 1", tone: "run" },
    });
    expect(cell(reviewing({ phaseLabel: "" })).live?.text).toBe("Reviewing");
  });
  it("says finishing up under three minutes", () => {
    expect(cell(reviewing({ finishesAt: iso(179) })).text).toBe("finishing up");
    expect(cell(reviewing({ finishesAt: iso(181) })).text).toBe("~3 min");
  });
  it("says running late when overdue or past the estimate", () => {
    expect(cell(reviewing({ overdue: true })).text).toBe("running late");
    expect(cell(reviewing({ finishesAt: iso(-1) })).text).toBe("running late");
    expect(cell(reviewing({ overdue: true, finishesAt: null })).text).toBe("running late");
  });
  it("says no estimate without one, and still says what is running", () => {
    expect(cell(reviewing({ finishesAt: null }))).toMatchObject({
      text: "no estimate",
      strong: false,
      dim: true,
      live: { text: "Reviewing · Phase 1" },
    });
  });
  it("shows the queued estimate, not bold, with a note and the queue position", () => {
    const bot: BotReview = {
      state: "queued",
      position: 3,
      startsAt: iso(891),
      finishesAt: iso(2815),
      reason: null,
      priority: false,
    };
    const c = cell(bot);
    expect(c).toMatchObject({ text: "~47 min", strong: false, live: { text: "Queued · #3", tone: "wait" } });
    expect(c.title).toMatch(/Estimated from the bot's median review time/);
    expect(cell({ ...bot, finishesAt: null, position: null })).toMatchObject({
      text: "no estimate",
      live: { text: "Queued" },
    });
  });
  it("shows the countdown, not the last review, while a review is live", () => {
    expect(cell(reviewing(), { lastReviewAt: iso(-620) }).text).toBe("~20 min");
  });
  it("shows when the last review came in, dim, with the exact time in the tooltip", () => {
    const c = cell({ state: "approved", at: iso(-9999) }, { lastReviewAt: iso(-620) });
    expect(c).toMatchObject({ text: "10 min ago", dim: true, live: null });
    expect(c.title).toMatch(/^Last review /);
    // Whatever the verdict, and even for a draft reviewed before.
    expect(cell({ state: "draft" }, { lastReviewAt: iso(-4 * 86400) }).text).toBe("4 d ago");
    expect(cell({ state: "other", title: "Cancelled" }, { lastReviewAt: iso(-1600) }).text).toBe("26 min ago");
  });
  it("shows a dash when the bot never reviewed", () => {
    expect(cell({ state: "commented", at: iso(-60) }).text).toBe("—");
    expect(cell({ state: "draft" })).toMatchObject({ text: "—", dim: true, title: null });
    expect(cell({ state: "none" }).text).toBe("—");
  });
});

describe("needsFastTick", () => {
  it("is true only for a countdown under two minutes", () => {
    expect(needsFastTick(reviewing({ finishesAt: iso(100) }), NOW)).toBe(true);
    expect(needsFastTick(reviewing({ finishesAt: iso(130) }), NOW)).toBe(false);
    expect(needsFastTick(reviewing({ finishesAt: null }), NOW)).toBe(false);
    expect(needsFastTick({ state: "approved", at: iso(-5) }, NOW)).toBe(false);
  });
});

describe("botStatusText", () => {
  it("labels every state", () => {
    expect(botStatusText(reviewing())).toBe("Reviewing · Phase 1");
    const queued: BotReview = { state: "queued", position: 3, startsAt: null, finishesAt: null, reason: null, priority: false };
    expect(botStatusText(queued)).toBe("Queued · #3");
    expect(botStatusText({ ...queued, position: null })).toBe("Queued");
    expect(botStatusText({ state: "approved", at: null })).toBe("Approved");
    expect(botStatusText({ state: "changesRequested", at: null })).toBe("Changes requested");
    expect(botStatusText({ state: "commented", at: null })).toBe("Comments only");
    expect(botStatusText({ state: "draft" })).toBe("Draft · not reviewed");
    expect(botStatusText({ state: "other", title: "Cancelled" })).toBe("Cancelled");
    expect(botStatusText({ state: "none" })).toBe("No bot review");
  });
});

describe("humanSummary", () => {
  it("picks the single most important fact", () => {
    expect(humanSummary(basePr())).toBeNull();
    expect(humanSummary(basePr({ reviewDecision: "reviewRequired" }))).toBe("needs 1 human review");
    expect(
      humanSummary(
        basePr({
          reviewDecision: "approved",
          humans: [
            { login: "shumkov", avatarUrl: null, state: "approved" },
            { login: "lklimek", avatarUrl: null, state: "requested" },
          ],
        }),
      ),
    ).toBe("approved by shumkov");
    expect(
      humanSummary(
        basePr({
          humans: [
            { login: "knst", avatarUrl: null, state: "commented" },
            { login: "UdjinM6", avatarUrl: null, state: "changesRequested" },
          ],
        }),
      ),
    ).toBe("changes requested by UdjinM6");
    expect(humanSummary(basePr({ humans: [{ login: "lklimek", avatarUrl: null, state: "requested" }] }))).toBe(
      "review requested from lklimek",
    );
  });
});

describe("subLineParts", () => {
  it("leaves out the round until the bot has posted a review", () => {
    for (const rounds of [null, 0]) {
      const parts = subLineParts(basePr({ rounds }), NOW, { showRepo: false });
      expect(parts.some((p) => p.startsWith("round"))).toBe(false);
    }
    expect(subLineParts(basePr({ rounds: 1 }), NOW, { showRepo: false })).toContain("round 1");
  });

  it("lists number, size, author, round, priority and the human summary", () => {
    const pr = basePr({
      isMine: false,
      author: "lklimek",
      rounds: 6,
      reviewDecision: "reviewRequired",
      bot: reviewing({ priority: true }),
    });
    expect(subLineParts(pr, NOW, { showRepo: false })).toEqual([
      "#6033",
      "+1132 −329",
      "lklimek",
      "round 6",
      "priority",
      "needs 1 human review",
    ]);
  });
  it("names an unrecognised bot state unless the status cell already does", () => {
    const other: BotReview = { state: "other", title: "Cancelled" };
    expect(subLineParts(basePr({ bot: other }), NOW, { showRepo: false })).not.toContain("Cancelled");
    const long: BotReview = { state: "other", title: "The review ended without a verdict" };
    expect(subLineParts(basePr({ bot: long }), NOW, { showRepo: false })).toContain(long.title);
    // Findings take the status cell, so the title moves to the sub-line.
    const findings = { ...ZERO_COUNTS, suggestion: { total: 1, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 } };
    expect(subLineParts(basePr({ bot: other, findingCounts: findings }), NOW, { showRepo: false })).toContain(
      "Cancelled",
    );
  });
  it("puts the milestone right after the size, and nothing without one", () => {
    const approved: BotReview = { state: "approved", at: null };
    const pr = basePr({ milestone: "v5.1.0", isMine: false, author: "lklimek", bot: approved });
    expect(subLineParts(pr, NOW, { showRepo: false })).toEqual(["#6033", "+1132 −329", "v5.1.0", "lklimek"]);
    expect(subLineParts(basePr({ bot: approved }), NOW, { showRepo: false })).toEqual(["#6033", "+1132 −329"]);
  });
  it("adds the repo and the queued start time", () => {
    const pr = basePr({
      bot: { state: "queued", position: 3, startsAt: iso(891), finishesAt: iso(2815), reason: null, priority: false },
    });
    expect(subLineParts(pr, NOW, { showRepo: true })).toEqual([
      "dashpay/platform",
      "#6033",
      "+1132 −329",
      "starts in ~15 min",
    ]);
  });
});

describe("nextStepRowLabel", () => {
  const queued: BotReview = { state: "queued", position: 3, startsAt: null, finishesAt: null, reason: null, priority: false };
  it("is the label, last on the sub-line", () => {
    const pr = basePr({ bot: { state: "approved", at: null }, nextStep: step("waiting-self-review", "Your move") });
    expect(nextStepRowLabel(pr)).toBe("Your move");
    expect(subLineParts(pr, NOW, { showRepo: false })).toEqual(["#6033", "+1132 −329", "Your move"]);
  });
  it("is null without a next step", () => {
    expect(nextStepRowLabel(basePr())).toBeNull();
  });
  it("leaves out Draft on a draft, whose title line starts with a Draft tag", () => {
    const draft = step("draft", "Draft");
    expect(nextStepRowLabel(basePr({ isDraft: true, bot: { state: "draft" }, nextStep: draft }))).toBeNull();
    // With findings from earlier reviews too: the tag says it whatever the status cell shows.
    const findings = { ...ZERO_COUNTS, blocking: { total: 2, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 } };
    expect(
      nextStepRowLabel(basePr({ isDraft: true, bot: { state: "draft" }, nextStep: draft, findingCounts: findings })),
    ).toBeNull();
    // The gate says draft but GitHub does not: no tag, so the line keeps it.
    expect(nextStepRowLabel(basePr({ isDraft: false, bot: { state: "none" }, nextStep: draft }))).toBe("Draft");
  });
  it("leaves out Waiting for bots while the bot is reviewing or queued", () => {
    const waiting = step("waiting-bots", "Waiting for bots");
    expect(nextStepRowLabel(basePr({ bot: reviewing(), nextStep: waiting }))).toBeNull();
    expect(nextStepRowLabel(basePr({ bot: queued, nextStep: waiting }))).toBeNull();
    expect(nextStepRowLabel(basePr({ bot: { state: "none" }, nextStep: waiting }))).toBe("Waiting for bots");
    expect(nextStepRowLabel(basePr({ bot: { state: "approved", at: null }, nextStep: waiting }))).toBe(
      "Waiting for bots",
    );
  });
  it("leaves out Your move while an agent works on the PR in T3 Code", () => {
    const yours = step("waiting-self-review", "Your move");
    const thread = (state: T3Thread["state"]): T3Thread => ({
      threadId: state,
      title: "Fix review findings",
      state,
      lastActiveAt: null,
      watching: false,
    });
    const working = basePr({ isMine: true, nextStep: yours, t3Threads: [thread("idle"), thread("working")] });
    expect(nextStepRowLabel(working)).toBeNull();
    expect(subLineParts(working, NOW, { showRepo: false })).not.toContain("Your move");
    // Waiting or idle threads leave it to the author again.
    expect(nextStepRowLabel(basePr({ isMine: true, nextStep: yours, t3Threads: [thread("waiting")] }))).toBe(
      "Your move",
    );
    expect(nextStepRowLabel(basePr({ isMine: true, nextStep: yours, t3Threads: [thread("idle")] }))).toBe("Your move");
    // Someone else's PR keeps "Author's move": the agent is not its author.
    const theirs = step("waiting-self-review", "Author's move");
    expect(nextStepRowLabel(basePr({ isMine: false, nextStep: theirs, t3Threads: [thread("working")] }))).toBe(
      "Author's move",
    );
    // Other stages stay.
    const ready = step("ready-to-merge", "Ready to merge");
    expect(nextStepRowLabel(basePr({ isMine: true, nextStep: ready, t3Threads: [thread("working")] }))).toBe(
      "Ready to merge",
    );
  });
  it("keeps other stages whatever the bot is doing", () => {
    expect(nextStepRowLabel(basePr({ bot: reviewing(), nextStep: step("ready-to-merge", "Ready to merge") }))).toBe(
      "Ready to merge",
    );
    expect(nextStepRowLabel(basePr({ bot: { state: "draft" }, nextStep: step("needs-rebase", "Needs rebase") }))).toBe(
      "Needs rebase",
    );
  });
});

describe("findingsText", () => {
  it("lists the non-zero kinds", () => {
    expect(findingsText({ blocking: 1, suggestion: 2, nitpick: 0 })).toBe("1 blocking · 2 suggestions");
    expect(findingsText({ blocking: 0, suggestion: 1, nitpick: 3 })).toBe("1 suggestion · 3 nitpicks");
    expect(findingsText({ blocking: 0, suggestion: 0, nitpick: 0 })).toBe("none");
  });
});
