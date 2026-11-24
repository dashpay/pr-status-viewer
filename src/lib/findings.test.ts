import { describe, expect, it } from "vitest";
import type { BotReview, FindingCounts, PullRequest, SeverityCount } from "../types";
import { mockSnapshot } from "../mock/snapshot";
import { bubble, countText, findingCountsText, severityText, statusCell, tally } from "./findings";

const sc = (total: number, agreed = 0, notAgreed = 0, coderabbit = 0, awaiting = 0): SeverityCount => ({
  total,
  agreed,
  notAgreed,
  awaiting,
  coderabbit,
});

/** Only awaiting re-review, plus any agreed and not agreed: [total, agreed, notAgreed, awaiting]. */
const aw = (total: number, agreed: number, notAgreed: number, awaiting: number): SeverityCount =>
  sc(total, agreed, notAgreed, 0, awaiting);

function counts(blocking = sc(0), suggestion = sc(0), nitpick = sc(0)): FindingCounts {
  return { blocking, suggestion, nitpick };
}

function pr(patch: Partial<PullRequest> = {}): PullRequest {
  const base = mockSnapshot(Date.parse("2026-11-22T12:00:00Z")).projects[0].prs[0];
  return { ...base, bot: { state: "approved", at: null }, lastReviewAt: null, findingCounts: counts(), ...patch };
}

/** What the bubble shows, as text: "rabbit down 1 up 1 / 2". */
function shown(b: ReturnType<typeof bubble>): string | null {
  if (!b) return null;
  const parts: string[] = b.coderabbit ? ["rabbit"] : [];
  if (b.notAgreed != null) parts.push(`down ${b.notAgreed}`);
  if (b.awaiting != null) parts.push(`wait ${b.awaiting}`);
  if (b.agreed != null) parts.push(`up ${b.agreed}`);
  if (b.notAgreed == null && b.awaiting == null && b.agreed == null) parts.push("0");
  return `${parts.join(" ")} / ${b.total}`;
}

describe("bubble", () => {
  it("shows the parts the user described", () => {
    // All settled with agreement.
    expect(shown(bubble("blocking", sc(2, 2, 0)))).toBe("up 2 / 2");
    // Mixed: thumbs-down first.
    expect(shown(bubble("blocking", sc(2, 1, 1)))).toBe("down 1 up 1 / 2");
    // Nothing settled.
    expect(shown(bubble("suggestion", sc(3)))).toBe("0 / 3");
    // Some agreed, none disputed.
    expect(shown(bubble("suggestion", sc(3, 1, 0)))).toBe("up 1 / 3");
    // Only not agreed.
    expect(shown(bubble("blocking", sc(2, 0, 1)))).toBe("down 1 / 2");
  });

  it("shows findings awaiting re-review between the thumbs", () => {
    // dashpay/platform#6056: every finding fixed and resolved, the bot not back yet.
    expect(shown(bubble("blocking", aw(5, 0, 0, 5)))).toBe("wait 5 / 5");
    expect(shown(bubble("suggestion", aw(6, 2, 1, 3)))).toBe("down 1 wait 3 up 2 / 6");
    expect(shown(bubble("blocking", aw(4, 1, 0, 2)))).toBe("wait 2 up 1 / 4");
    expect(shown(bubble("blocking", aw(4, 0, 1, 2)))).toBe("down 1 wait 2 / 4");
    expect(shown(bubble("blocking", sc(2, 0, 0, 2, 1)))).toBe("rabbit wait 1 / 2");
    // None awaiting: no hourglass.
    expect(bubble("blocking", sc(2, 1, 1))?.awaiting).toBeNull();
  });

  it("has no bubble for a severity without findings", () => {
    expect(bubble("blocking", sc(0))).toBeNull();
  });

  it("is quiet only when every finding is settled, either way", () => {
    expect(bubble("blocking", sc(2, 2, 0))?.settled).toBe(true);
    expect(bubble("blocking", sc(2, 1, 1))?.settled).toBe(true);
    expect(bubble("blocking", sc(2, 0, 2))?.settled).toBe(true);
    expect(bubble("blocking", sc(3, 1, 1))).toMatchObject({ settled: false, open: 1 });
    expect(bubble("suggestion", sc(3))).toMatchObject({ settled: false, open: 3 });
  });

  it("is quiet when the rest await re-review: those are not the author's move", () => {
    expect(bubble("blocking", aw(5, 0, 0, 5))).toMatchObject({ settled: true, open: 0, awaiting: 5 });
    expect(bubble("suggestion", aw(6, 2, 1, 3))).toMatchObject({ settled: true, open: 0 });
    // One still open: loud.
    expect(bubble("blocking", aw(5, 0, 0, 4))).toMatchObject({ settled: false, open: 1, awaiting: 4 });
  });

  it("names the counts in words, singular and plural", () => {
    expect(bubble("blocking", sc(2, 1, 1))?.label).toBe(
      "2 blockers in the last review round: 1 agreed, 1 not agreed, 0 open",
    );
    expect(bubble("blocking", sc(1))?.label).toBe("1 blocker in the last review round: 0 agreed, 0 not agreed, 1 open");
    expect(bubble("suggestion", sc(1, 1))?.label).toBe(
      "1 suggestion in the last review round: 1 agreed, 0 not agreed, 0 open",
    );
    expect(bubble("suggestion", sc(92, 87, 3))?.label).toBe(
      "92 suggestions in the last review round: 87 agreed, 3 not agreed, 2 open",
    );
  });

  it("names findings awaiting re-review only when there are some", () => {
    expect(bubble("blocking", aw(5, 0, 0, 5))?.label).toBe(
      "5 blockers in the last review round: 0 agreed, 0 not agreed, 5 awaiting re-review, 0 open",
    );
    expect(bubble("suggestion", aw(6, 2, 1, 3))?.label).toBe(
      "6 suggestions in the last review round: 2 agreed, 1 not agreed, 3 awaiting re-review, 0 open",
    );
    expect(bubble("blocking", sc(3, 0, 0, 1, 1))?.label).toBe(
      "3 blockers in the last review round: 0 agreed, 0 not agreed, 1 awaiting re-review, 2 open. 1 from CodeRabbit.",
    );
    expect(bubble("blocking", aw(2, 1, 1, 0))?.label).not.toContain("awaiting");
  });

  it("shows a rabbit only when CodeRabbit has findings in the total", () => {
    expect(bubble("blocking", sc(2, 1, 1))?.coderabbit).toBe(false);
    expect(shown(bubble("blocking", sc(2, 1, 1, 1)))).toBe("rabbit down 1 up 1 / 2");
    // Every finding CodeRabbit's.
    expect(shown(bubble("suggestion", sc(3, 0, 0, 3)))).toBe("rabbit 0 / 3");
    // More CodeRabbit findings than the total: kept in range.
    expect(bubble("suggestion", sc(2, 0, 0, 5))?.label).toMatch(/\. 2 from CodeRabbit\.$/);
    expect(bubble("suggestion", sc(2, 0, 0, -1))?.coderabbit).toBe(false);
  });

  it("names CodeRabbit's share in words, and says nothing of it when it has none", () => {
    expect(bubble("blocking", sc(2, 1, 1, 1))?.label).toBe(
      "2 blockers in the last review round: 1 agreed, 1 not agreed, 0 open. 1 from CodeRabbit.",
    );
    expect(bubble("blocking", sc(1, 0, 0, 1))?.label).toBe(
      "1 blocker in the last review round: 0 agreed, 0 not agreed, 1 open. 1 from CodeRabbit.",
    );
    expect(bubble("suggestion", sc(5, 2, 1, 3))?.label).toBe(
      "5 suggestions in the last review round: 2 agreed, 1 not agreed, 2 open. 3 from CodeRabbit.",
    );
    expect(bubble("suggestion", sc(1, 1, 0, 0))?.label).not.toContain("CodeRabbit");
  });

  it("tolerates counts from a backend without the CodeRabbit field", () => {
    const old = { total: 2, agreed: 1, notAgreed: 0 } as SeverityCount;
    expect(bubble("blocking", old)).toMatchObject({ coderabbit: false, total: 2 });
    expect(severityText(old)).toBe("1 of 2 settled");
  });

  it("tolerates counts from a backend without the awaiting field", () => {
    const old = { total: 3, agreed: 1, notAgreed: 1, coderabbit: 0 } as SeverityCount;
    expect(tally(old)).toEqual({ total: 3, agreed: 1, notAgreed: 1, awaiting: 0, open: 1, coderabbit: 0 });
    expect(bubble("blocking", old)).toMatchObject({ awaiting: null, open: 1, settled: false });
    expect(bubble("blocking", old)?.label).toBe("3 blockers in the last review round: 1 agreed, 1 not agreed, 1 open");
    expect(severityText(old)).toBe("2 of 3 settled (1 not agreed)");
  });

  it("squeezes wide numbers so they stay inside the slot", () => {
    expect(bubble("blocking", sc(2, 1, 1))?.fit).toBe("normal");
    expect(bubble("suggestion", sc(92, 87, 3))?.fit).toBe("normal");
    expect(bubble("suggestion", sc(120, 99, 12))?.fit).toBe("tight");
    expect(bubble("suggestion", sc(999, 456, 123))?.fit).toBe("squeeze");
    // Without thumbs even three-digit numbers fit as they are.
    expect(bubble("suggestion", sc(250))?.fit).toBe("normal");
    // The rabbit has its own room in the slot, so it does not change the fit.
    expect(bubble("suggestion", sc(92, 87, 3, 40))?.fit).toBe("normal");
    expect(bubble("suggestion", sc(999, 456, 123, 999))?.fit).toBe("squeeze");
  });

  it("counts the hourglass like a thumb when fitting", () => {
    // "⏳5 / 5" is 3.5.
    expect(bubble("blocking", aw(5, 0, 0, 5))?.fit).toBe("normal");
    // "👎1 ⏳3 👍2 / 6" is 8.5.
    expect(bubble("suggestion", aw(6, 2, 1, 3))?.fit).toBe("tight");
    // "⏳3 👍87 / 92" is 8, as "👎3 👍87 / 92".
    expect(bubble("suggestion", aw(92, 87, 0, 3))?.fit).toBe("normal");
    // "👎12 ⏳5 👍70 / 92" is 11.5; "👎33 ⏳33 👍33 / 99" is 12.5.
    expect(bubble("suggestion", aw(92, 70, 12, 5))?.fit).toBe("squeeze");
    expect(bubble("suggestion", aw(99, 33, 33, 33))?.fit).toBe("squeeze");
    // Three counts and a three-digit total, "👎33 ⏳33 👍33 / 100", is 13.5.
    expect(bubble("suggestion", aw(100, 33, 33, 33))?.fit).toBe("crush");
    // The widest possible, "👎123 ⏳123 👍456 / 999", is 16.5.
    expect(bubble("suggestion", sc(999, 456, 123, 999, 123))?.fit).toBe("crush");
  });
});

describe("countText", () => {
  it("is exact below 1000 and never longer than three characters", () => {
    expect(countText(0)).toBe("0");
    expect(countText(999)).toBe("999");
    expect(countText(1000)).toBe("1k");
    expect(countText(12_345)).toBe("12k");
    expect(countText(5_000_000)).toBe("999k");
  });
});

describe("tally", () => {
  it("keeps inconsistent counts in range", () => {
    expect(tally(sc(2, 3, 1))).toEqual({ total: 2, agreed: 2, notAgreed: 0, awaiting: 0, open: 0, coderabbit: 0 });
    expect(tally(sc(3, 1, 5, 4))).toEqual({ total: 3, agreed: 1, notAgreed: 2, awaiting: 0, open: 0, coderabbit: 3 });
    expect(tally(sc(-1, -1, -1, -1, -1))).toEqual({
      total: 0,
      agreed: 0,
      notAgreed: 0,
      awaiting: 0,
      open: 0,
      coderabbit: 0,
    });
  });

  it("takes awaiting from what agreed and not agreed leave, never past the total", () => {
    expect(tally(aw(6, 2, 1, 3))).toMatchObject({ agreed: 2, notAgreed: 1, awaiting: 3, open: 0 });
    expect(tally(aw(6, 2, 1, 9))).toMatchObject({ agreed: 2, notAgreed: 1, awaiting: 3, open: 0 });
    expect(tally(aw(3, 2, 5, 4))).toMatchObject({ agreed: 2, notAgreed: 1, awaiting: 0, open: 0 });
    expect(tally(aw(5, 0, 0, 2))).toMatchObject({ awaiting: 2, open: 3 });
    expect(tally(aw(5, 0, 0, Number.NaN))).toMatchObject({ awaiting: 0, open: 5 });
    expect(bubble("blocking", aw(3, 2, 5, 4))?.awaiting).toBeNull();
  });
});

describe("statusCell", () => {
  it("shows bubbles in both slots, or an empty slot for a severity without findings", () => {
    const both = statusCell(pr({ findingCounts: counts(sc(2, 1, 1), sc(3)) }));
    expect(both.kind).toBe("bubbles");
    if (both.kind !== "bubbles") return;
    expect(both.blocking?.severity).toBe("blocking");
    expect(both.suggestion?.severity).toBe("suggestion");

    const blockersOnly = statusCell(pr({ findingCounts: counts(sc(1)) }));
    expect(blockersOnly).toMatchObject({ kind: "bubbles", suggestion: null });
    const suggestionsOnly = statusCell(pr({ findingCounts: counts(sc(0), sc(3, 1)) }));
    expect(suggestionsOnly).toMatchObject({ kind: "bubbles", blocking: null });
  });

  it("gives nitpicks no bubble", () => {
    const cell = statusCell(pr({ lastReviewAt: "2026-11-22T11:00:00Z", findingCounts: counts(sc(0), sc(0), sc(4)) }));
    expect(cell).toEqual({ kind: "note", text: "No findings", tone: "dim" });
  });

  it("says No findings once the bot has reviewed", () => {
    expect(statusCell(pr({ lastReviewAt: "2026-11-22T11:00:00Z" }))).toMatchObject({ text: "No findings" });
    expect(statusCell(pr({ bot: { state: "commented", at: null } }))).toMatchObject({ text: "No findings" });
    // Under review again after a clean review.
    const reviewing = mockSnapshot().projects[0].prs[0].bot;
    expect(statusCell(pr({ bot: reviewing, lastReviewAt: "2026-11-22T11:00:00Z" }))).toMatchObject({
      text: "No findings",
    });
  });

  it("keeps the texts for drafts, PRs without a bot review, and first reviews", () => {
    expect(statusCell(pr({ bot: { state: "draft" } }))).toEqual({
      kind: "note",
      // The row's Draft tag already says draft.
      text: "Not reviewed",
      tone: "draft",
    });
    expect(statusCell(pr({ bot: { state: "none" } }))).toMatchObject({ text: "No bot review", tone: "dim" });
    const queued: BotReview = { state: "queued", position: 2, startsAt: null, finishesAt: null, reason: null, priority: false };
    expect(statusCell(pr({ bot: queued }))).toMatchObject({ text: "First review" });
    expect(statusCell(pr({ bot: { state: "other", title: "Cancelled" } }))).toMatchObject({ text: "Cancelled" });
  });

  it("shows earlier findings on a draft", () => {
    expect(statusCell(pr({ bot: { state: "draft" }, findingCounts: counts(sc(1)) })).kind).toBe("bubbles");
  });
});

describe("detail text", () => {
  it("spells out each severity", () => {
    expect(severityText(sc(0))).toBe("none");
    expect(severityText(sc(3))).toBe("0 of 3 settled");
    expect(severityText(sc(2, 2, 0))).toBe("2 of 2 settled");
    expect(severityText(sc(2, 1, 1))).toBe("2 of 2 settled (1 not agreed)");
    expect(findingCountsText(counts(sc(2, 1, 1), sc(3), sc(4, 1)))).toBe(
      "Blockers 2 of 2 settled (1 not agreed) · Suggestions 0 of 3 settled · Nitpicks 1 of 4 settled",
    );
  });

  it("names findings awaiting re-review after the settled count", () => {
    expect(severityText(aw(5, 0, 0, 5))).toBe("0 of 5 settled, 5 awaiting re-review");
    expect(severityText(aw(6, 2, 1, 3))).toBe("3 of 6 settled (1 not agreed), 3 awaiting re-review");
    expect(severityText(sc(4, 1, 1, 1, 2))).toBe(
      "2 of 4 settled (1 not agreed), 2 awaiting re-review, 1 from CodeRabbit",
    );
    expect(findingCountsText(counts(aw(5, 0, 0, 5), aw(5, 0, 0, 5), sc(2, 1)))).toBe(
      "Blockers 0 of 5 settled, 5 awaiting re-review · Suggestions 0 of 5 settled, 5 awaiting re-review · Nitpicks 1 of 2 settled",
    );
  });

  it("names CodeRabbit's share of each severity that has some", () => {
    expect(severityText(sc(2, 0, 1, 1))).toBe("1 of 2 settled (1 not agreed), 1 from CodeRabbit");
    expect(severityText(sc(3, 0, 0, 3))).toBe("0 of 3 settled, 3 from CodeRabbit");
    expect(findingCountsText(counts(sc(2, 0, 1, 1), sc(3), sc(0)))).toBe(
      "Blockers 1 of 2 settled (1 not agreed), 1 from CodeRabbit · Suggestions 0 of 3 settled · Nitpicks none",
    );
  });
});
