import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import type { PullRequest, SelfReviewState } from "../types";
import {
  gateAsksForSelfReview,
  postedText,
  rowNextStepWithCue,
  selfReviewButtonLabel,
  selfReviewPaletteLabel,
  splitCommand,
} from "./selfReview";

const NOW = Date.parse("2026-11-22T12:00:00Z");
const base = mockSnapshot(NOW).projects[0].prs[0];
const withState = (state: SelfReviewState, number = 6038): PullRequest => ({
  ...base,
  number,
  selfReview: { state, postedAt: null },
});

describe("selfReviewButtonLabel", () => {
  it("offers the post for wanted and possible, Post again once posted, nothing otherwise", () => {
    expect(selfReviewButtonLabel("wanted")).toBe("Post /self-reviewed");
    expect(selfReviewButtonLabel("possible")).toBe("Post /self-reviewed");
    expect(selfReviewButtonLabel("posted")).toBe("Post again");
    expect(selfReviewButtonLabel("unavailable")).toBeNull();
  });
});

describe("postedText", () => {
  it("says when, and that the gate has not moved on yet", () => {
    expect(postedText("2026-11-22T11:57:00Z", NOW)).toBe("Self-review posted 3 min ago, waiting for the gate");
    expect(postedText("2026-11-22T11:59:40Z", NOW)).toBe("Self-review posted just now, waiting for the gate");
  });

  it("leaves the time out when there is none", () => {
    expect(postedText(null, NOW)).toBe("Self-review posted, waiting for the gate");
    expect(postedText("not a time", NOW)).toBe("Self-review posted, waiting for the gate");
  });
});

describe("gateAsksForSelfReview", () => {
  it("is true only when the gate's sentence names the command", () => {
    expect(gateAsksForSelfReview("Bots are done - your move: post /self-reviewed.")).toBe(true);
    expect(gateAsksForSelfReview("Post /Self-Review when ready")).toBe(true);
    expect(gateAsksForSelfReview("Waiting for bot review - thepastaclaw not yet.")).toBe(false);
    expect(gateAsksForSelfReview(null)).toBe(false);
  });
});

describe("selfReviewPaletteLabel", () => {
  it("names the PR, says again once posted, and offers nothing when unavailable", () => {
    expect(selfReviewPaletteLabel(withState("wanted"))).toBe("Post /self-reviewed on #6038");
    expect(selfReviewPaletteLabel(withState("possible", 6033))).toBe("Post /self-reviewed on #6033");
    expect(selfReviewPaletteLabel(withState("posted"))).toBe("Post /self-reviewed again on #6038");
    expect(selfReviewPaletteLabel(withState("unavailable"))).toBeNull();
  });
});

describe("rowNextStepWithCue", () => {
  it("adds the self-review to the label only when the gate wants it", () => {
    expect(rowNextStepWithCue(withState("wanted"), "Your move")).toBe("Your move: self-review");
    for (const state of ["possible", "posted", "unavailable"] as const) {
      expect(rowNextStepWithCue(withState(state), "Your move")).toBe("Your move");
    }
  });
});

describe("splitCommand", () => {
  it("marks each occurrence of the command", () => {
    expect(splitCommand("Bots are done - your move: post /self-reviewed.")).toEqual([
      { text: "Bots are done - your move: post ", command: false },
      { text: "/self-reviewed", command: true },
      { text: ".", command: false },
    ]);
    expect(splitCommand("/self-reviewed abc")).toEqual([
      { text: "/self-reviewed", command: true },
      { text: " abc", command: false },
    ]);
    expect(splitCommand("Refresh")).toEqual([{ text: "Refresh", command: false }]);
    expect(splitCommand("")).toEqual([]);
  });
});
