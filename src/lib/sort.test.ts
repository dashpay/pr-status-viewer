import { describe, expect, it } from "vitest";
import type { BotReview, PullRequest } from "../types";
import { mockSnapshot } from "../mock/snapshot";
import { sortPrs } from "./sort";

const NOW = Date.parse("2026-11-22T12:00:00Z");
const iso = (offsetSec: number) => new Date(NOW + offsetSec * 1000).toISOString();

function pr(id: string, bot: BotReview, lastReview: number | null, updated: number): PullRequest {
  const base = mockSnapshot(NOW).projects[0].prs[0];
  return {
    ...base,
    id,
    bot,
    lastReviewAt: lastReview == null ? null : iso(-lastReview),
    updatedAt: iso(-updated),
  };
}

const queued = (finishIn: number | null): BotReview => ({
  state: "queued",
  position: 1,
  startsAt: null,
  finishesAt: finishIn == null ? null : iso(finishIn),
  reason: null,
  priority: false,
});

describe("sortPrs: finishes first", () => {
  it("puts live reviews by finish time, then reviewed PRs newest review first, then the rest by update", () => {
    const prs = [
      pr("old-review", { state: "approved", at: iso(-10) }, 9000, 5),
      pr("never", { state: "none" }, null, 1),
      pr("queued-late", queued(3000), 600, 900),
      pr("new-review", { state: "changesRequested", at: iso(-99999) }, 60, 50),
      pr("queued-soon", queued(300), null, 900),
      pr("draft-reviewed", { state: "draft" }, 3000, 2),
      pr("draft-never", { state: "draft" }, null, 3),
    ];
    expect(sortPrs(prs, "finishesFirst").map((p) => p.id)).toEqual([
      "queued-soon",
      "queued-late",
      "new-review",
      "draft-reviewed",
      "old-review",
      "never",
      "draft-never",
    ]);
  });

  it("orders by the last review, not the verdict's own time", () => {
    const prs = [
      pr("verdict-newer", { state: "approved", at: iso(-10) }, 5000, 1),
      pr("review-newer", { state: "approved", at: iso(-9000) }, 100, 1),
    ];
    expect(sortPrs(prs, "finishesFirst").map((p) => p.id)).toEqual(["review-newer", "verdict-newer"]);
  });

  it("puts a live review without an estimate among the rest", () => {
    const prs = [pr("reviewed", { state: "approved", at: null }, 100, 50), pr("queued-no-estimate", queued(null), 10, 1)];
    expect(sortPrs(prs, "finishesFirst").map((p) => p.id)).toEqual(["reviewed", "queued-no-estimate"]);
  });
});
