import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import type { PullRequest } from "../types";
import {
  applySwitches,
  createdRecently,
  draftCount,
  draftShown,
  hiddenNote,
  RECENT_MS,
  recentCount,
  recentShown,
  reposOf,
  showDraftsSwitch,
  showRecentSwitch,
} from "./filters";

const NOW = Date.parse("2026-11-22T12:00:00Z");
const base = mockSnapshot(NOW).projects[0].prs[0];
const HOUR = 3600 * 1000;
const ago = (ms: number) => new Date(NOW - ms).toISOString();
const pr = (
  id: string,
  title: string,
  milestone: string | null,
  repo = "o/r",
  isDraft = false,
  createdAt = ago(10 * 24 * HOUR),
): PullRequest => ({
  ...base,
  id,
  repo,
  title,
  milestone,
  isDraft,
  createdAt,
});

describe("applySwitches", () => {
  const prs = [
    pr("a", "fix(sdk): a", "v1"),
    pr("b", "fix(sdk): b", "v2"),
    pr("c", "fix(drive): c", "v2"),
    pr("d", "fix(sdk, drive): d", "v1"),
    pr("e", "fix(sdk): e", "v1", "x/y"),
  ];

  it("lets everything through with no switches off", () => {
    const r = applySwitches(prs, [], {});
    expect(r.visible).toHaveLength(5);
    expect(r.byDraft).toEqual([]);
    expect(r.byAge).toEqual([]);
    expect(r.byMilestone).toEqual([]);
    expect(r.byArea).toEqual([]);
  });

  it("lets older pull requests through unless only recent ones are listed", () => {
    const withNew = [...prs, pr("n", "fix(sdk): n", "v1", "o/r", false, ago(HOUR))];
    expect(applySwitches(withNew, [], {}, true, false, NOW).visible).toHaveLength(6);
    const r = applySwitches(withNew, [], {}, true, true, NOW);
    expect(r.visible.map((p) => p.id)).toEqual(["n"]);
    expect(r.byAge.map((p) => p.id)).toEqual(["a", "b", "c", "d", "e"]);
    // Later, the new one is no longer recent either.
    const later = applySwitches(withNew, [], {}, true, true, NOW + RECENT_MS);
    expect(later.visible).toEqual([]);
    expect(later.byAge).toHaveLength(6);
  });

  it("counts each pull request once: drafts, then age, then milestone, then area", () => {
    const fresh = ago(HOUR);
    const mixed = [
      pr("d1", "fix(sdk): old draft in hidden milestone", "v2", "o/r", true),
      pr("d2", "fix(drive): new draft", "v1", "o/r", true, fresh),
      pr("o1", "fix(sdk): old, hidden milestone and area", "v2"),
      pr("o2", "fix(drive): old, otherwise shown", "v1"),
      pr("m1", "fix(drive): new, hidden milestone", "v2", "o/r", false, fresh),
      pr("a1", "fix(sdk): new, hidden area", "v1", "o/r", false, fresh),
      pr("v1", "fix(drive): new, shown", "v1", "o/r", false, fresh),
    ];
    const r = applySwitches(mixed, ["v2"], { "o/r": ["sdk"] }, false, true, NOW);
    expect(r.byDraft.map((p) => p.id)).toEqual(["d1", "d2"]);
    expect(r.byAge.map((p) => p.id)).toEqual(["o1", "o2"]);
    expect(r.byMilestone.map((p) => p.id)).toEqual(["m1"]);
    expect(r.byArea.map((p) => p.id)).toEqual(["a1"]);
    expect(r.visible.map((p) => p.id)).toEqual(["v1"]);
    // With drafts shown, the old draft falls to age and the new one shows.
    const shown = applySwitches(mixed, ["v2"], { "o/r": ["sdk"] }, true, true, NOW);
    expect(shown.byAge.map((p) => p.id)).toEqual(["d1", "o1", "o2"]);
    expect(shown.visible.map((p) => p.id)).toEqual(["d2", "v1"]);
    // With the switch off, the old ones fall to milestone and area.
    const all = applySwitches(mixed, ["v2"], { "o/r": ["sdk"] }, true, false, NOW);
    expect(all.byAge).toEqual([]);
    expect(all.byMilestone.map((p) => p.id)).toEqual(["d1", "o1", "m1"]);
  });

  it("lets drafts through unless the drafts switch is off", () => {
    const withDrafts = [...prs, pr("f", "fix(sdk): f", "v1", "o/r", true)];
    expect(applySwitches(withDrafts, [], {}).visible.map((p) => p.id)).toContain("f");
    expect(applySwitches(withDrafts, [], {}, true).byDraft).toEqual([]);
    const r = applySwitches(withDrafts, [], {}, false);
    expect(r.byDraft.map((p) => p.id)).toEqual(["f"]);
    expect(r.visible).toHaveLength(5);
  });

  it("counts each pull request once: drafts first, then milestone, then area", () => {
    const mixed = [
      pr("d1", "fix(sdk): draft in hidden milestone and area", "v2", "o/r", true),
      pr("d2", "fix(drive): draft in hidden milestone", "v2", "o/r", true),
      pr("d3", "fix(sdk): draft in hidden area", "v1", "o/r", true),
      pr("d4", "fix(drive): draft otherwise shown", "v1", "o/r", true),
      pr("m1", "fix(sdk): hidden milestone and area", "v2"),
      pr("a1", "fix(sdk): hidden area", "v1"),
      pr("v1", "fix(drive): shown", "v1"),
    ];
    const r = applySwitches(mixed, ["v2"], { "o/r": ["sdk"] }, false);
    expect(r.byDraft.map((p) => p.id)).toEqual(["d1", "d2", "d3", "d4"]);
    expect(r.byMilestone.map((p) => p.id)).toEqual(["m1"]);
    expect(r.byArea.map((p) => p.id)).toEqual(["a1"]);
    expect(r.visible.map((p) => p.id)).toEqual(["v1"]);
    const total = r.byDraft.length + r.byMilestone.length + r.byArea.length + r.visible.length;
    expect(total).toBe(mixed.length);
    // With drafts shown, the drafts fall to the next cause that hides them.
    const shown = applySwitches(mixed, ["v2"], { "o/r": ["sdk"] }, true);
    expect(shown.byMilestone.map((p) => p.id)).toEqual(["d1", "d2", "m1"]);
    expect(shown.byArea.map((p) => p.id)).toEqual(["d3", "a1"]);
    expect(shown.visible.map((p) => p.id)).toEqual(["d4", "v1"]);
  });

  it("counts a pull request hidden by both under milestone only", () => {
    const r = applySwitches(prs, ["v2"], { "o/r": ["sdk"] });
    expect(r.byMilestone.map((p) => p.id)).toEqual(["b", "c"]);
    expect(r.byArea.map((p) => p.id)).toEqual(["a"]);
    // d still has drive on; e is in another repo.
    expect(r.visible.map((p) => p.id)).toEqual(["d", "e"]);
  });
});

describe("hiddenNote", () => {
  it("names each cause only when it hides something", () => {
    expect(hiddenNote({ byDraft: 0, byMilestone: 0, byArea: 0 })).toBeNull();
    expect(hiddenNote({ byDraft: 0, byMilestone: 4, byArea: 0 })).toBe("4 hidden by milestone");
    expect(hiddenNote({ byDraft: 0, byMilestone: 0, byArea: 2 })).toBe("2 hidden by area");
    expect(hiddenNote({ byDraft: 0, byMilestone: 4, byArea: 2 })).toBe("4 hidden by milestone · 2 hidden by area");
  });
  it("puts drafts first, singular for one", () => {
    expect(hiddenNote({ byDraft: 1, byMilestone: 0, byArea: 0 })).toBe("1 draft hidden");
    expect(hiddenNote({ byDraft: 3, byMilestone: 0, byArea: 0 })).toBe("3 drafts hidden");
    expect(hiddenNote({ byDraft: 3, byMilestone: 4, byArea: 1 })).toBe("3 drafts hidden · 4 hidden by milestone · 1 hidden by area");
    expect(hiddenNote({ byDraft: 2, byMilestone: 0, byArea: 1 })).toBe("2 drafts hidden · 1 hidden by area");
  });
  it("puts older pull requests after drafts and before milestones", () => {
    expect(hiddenNote({ byDraft: 0, byAge: 0, byMilestone: 4, byArea: 0 })).toBe("4 hidden by milestone");
    expect(hiddenNote({ byDraft: 0, byAge: 1, byMilestone: 0, byArea: 0 })).toBe("1 older than 3 days hidden");
    expect(hiddenNote({ byDraft: 0, byAge: 5, byMilestone: 0, byArea: 0 })).toBe("5 older than 3 days hidden");
    expect(hiddenNote({ byDraft: 3, byAge: 5, byMilestone: 4, byArea: 1 })).toBe(
      "3 drafts hidden · 5 older than 3 days hidden · 4 hidden by milestone · 1 hidden by area",
    );
  });
});

describe("created recently", () => {
  it("is the 3 days before now, inclusive", () => {
    expect(createdRecently(pr("a", "x", null, "o/r", false, ago(HOUR)), NOW)).toBe(true);
    expect(createdRecently(pr("a", "x", null, "o/r", false, ago(RECENT_MS)), NOW)).toBe(true);
    expect(createdRecently(pr("a", "x", null, "o/r", false, ago(RECENT_MS + 1000)), NOW)).toBe(false);
    expect(createdRecently(pr("a", "x", null, "o/r", false, ago(10 * 24 * HOUR)), NOW)).toBe(false);
    expect(RECENT_MS).toBe(72 * HOUR);
  });

  it("counts a pull request whose date cannot be read as not recent", () => {
    expect(createdRecently(pr("a", "x", null, "o/r", false, "not a date"), NOW)).toBe(false);
    expect(createdRecently(pr("a", "x", null, "o/r", false, ""), NOW)).toBe(false);
  });

  it("lets everything through while the switch is off", () => {
    const old = pr("a", "x", null);
    expect(recentShown(old, false, NOW)).toBe(true);
    expect(recentShown(old, true, NOW)).toBe(false);
  });

  it("counts recent pull requests and decides on the switch", () => {
    const list = [pr("a", "x", null), pr("b", "x", null, "o/r", false, ago(HOUR)), pr("c", "x", null, "o/r", true, ago(2 * HOUR))];
    expect(recentCount(list, NOW)).toBe(2);
    expect(recentCount([], NOW)).toBe(0);
    // Shown while the view has a recent pull request, or while the switch is on.
    expect(showRecentSwitch(2, false)).toBe(true);
    expect(showRecentSwitch(0, false)).toBe(false);
    expect(showRecentSwitch(0, true)).toBe(true);
  });
});

describe("drafts", () => {
  it("counts drafts and decides on the switch", () => {
    const list = [pr("a", "x", null), pr("b", "x", null, "o/r", true), pr("c", "x", null, "o/r", true)];
    expect(draftCount(list)).toBe(2);
    expect(draftCount([])).toBe(0);
    expect(draftShown(list[1], true)).toBe(true);
    expect(draftShown(list[1], false)).toBe(false);
    expect(draftShown(list[0], false)).toBe(true);
    // Shown while the view has a draft, or while drafts are hidden.
    expect(showDraftsSwitch(2, true)).toBe(true);
    expect(showDraftsSwitch(0, true)).toBe(false);
    expect(showDraftsSwitch(0, false)).toBe(true);
  });
});

describe("reposOf", () => {
  it("lists each repo once in order", () => {
    expect(reposOf([pr("a", "x", null, "b/b"), pr("b", "x", null, "a/a"), pr("c", "x", null, "b/b")])).toEqual([
      "b/b",
      "a/a",
    ]);
  });
});
