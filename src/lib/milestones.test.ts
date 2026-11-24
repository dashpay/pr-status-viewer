import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import type { PullRequest } from "../types";
import {
  NO_MILESTONE,
  compareMilestones,
  filterByMilestone,
  milestoneChips,
  showMilestoneRow,
  soloToggles,
} from "./milestones";

const template = mockSnapshot(0).projects[0].prs[0];
let n = 0;
function pr(milestone: string | null): PullRequest {
  n += 1;
  return { ...template, id: `acme/app#${n}`, number: n, milestone };
}

describe("compareMilestones", () => {
  it("orders versions numerically, ignoring a leading v", () => {
    const titles = ["v6.0.0", "v5.10.0", "v5.2.0", "v5.1.0", "25", "24.1", "5.3.0"];
    expect([...titles].sort(compareMilestones)).toEqual(["v5.1.0", "v5.2.0", "5.3.0", "v5.10.0", "v6.0.0", "24.1", "25"]);
    expect(compareMilestones("v5.2.0", "v5.10.0")).toBeLessThan(0);
    expect(compareMilestones("v5.10.0", "v6.0.0")).toBeLessThan(0);
    expect(compareMilestones("24.1", "25")).toBeLessThan(0);
    expect(compareMilestones("v5.3.0", "5.2.0")).toBeGreaterThan(0);
    expect(compareMilestones("5.3.0", "v5.10.0")).toBeLessThan(0);
  });

  it("sorts plain names alphabetically and stays stable on ties", () => {
    expect(["Backlog", "Alpha"].sort(compareMilestones)).toEqual(["Alpha", "Backlog"]);
    expect(compareMilestones("v5.1.0", "5.1.0")).not.toBe(0);
    expect(compareMilestones("v5.1.0", "v5.1.0")).toBe(0);
  });
});

describe("milestoneChips", () => {
  it("lists each milestone once with its count, in version order, no milestone last", () => {
    const prs = [pr(null), pr("v5.10.0"), pr("v5.2.0"), pr("v6.0.0"), pr("v5.2.0"), pr(null), pr("v5.1.0")];
    expect(milestoneChips(prs)).toEqual([
      { key: "v5.1.0", label: "v5.1.0", count: 1 },
      { key: "v5.2.0", label: "v5.2.0", count: 2 },
      { key: "v5.10.0", label: "v5.10.0", count: 1 },
      { key: "v6.0.0", label: "v6.0.0", count: 1 },
      { key: NO_MILESTONE, label: "No milestone", count: 2 },
    ]);
  });

  it("leaves out no milestone when every PR has one, and is empty for no PRs", () => {
    expect(milestoneChips([pr("25"), pr("24.1")]).map((c) => c.key)).toEqual(["24.1", "25"]);
    expect(milestoneChips([])).toEqual([]);
  });
});

describe("filterByMilestone", () => {
  const a = pr("v5.1.0");
  const b = pr("v5.2.0");
  const c = pr(null);
  it("keeps everything when nothing is switched off", () => {
    expect(filterByMilestone([a, b, c], [])).toEqual([a, b, c]);
  });
  it("hides switched-off titles", () => {
    expect(filterByMilestone([a, b, c], ["v5.2.0"])).toEqual([a, c]);
  });
  it("hides PRs without a milestone under NO_MILESTONE", () => {
    expect(filterByMilestone([a, b, c], [NO_MILESTONE])).toEqual([a, b]);
    expect(filterByMilestone([a, b, c], [NO_MILESTONE, "v5.1.0", "v5.2.0"])).toEqual([]);
  });
  it("ignores switched-off titles that are not present", () => {
    expect(filterByMilestone([a, c], ["v9.9.9"])).toEqual([a, c]);
  });
});

describe("showMilestoneRow", () => {
  const chips = (...keys: string[]) => keys.map((key) => ({ key, label: key || "No milestone", count: 1 }));
  it("hides the row when no PR has a milestone", () => {
    expect(showMilestoneRow(chips(NO_MILESTONE), [])).toBe(false);
    expect(showMilestoneRow([], [])).toBe(false);
  });
  it("hides the row for a single milestone with nothing unassigned", () => {
    expect(showMilestoneRow(chips("24.1"), [])).toBe(false);
  });
  it("shows the row when there is a choice", () => {
    expect(showMilestoneRow(chips("24.1", NO_MILESTONE), [])).toBe(true);
    expect(showMilestoneRow(chips("v5.1.0", "v5.2.0"), [])).toBe(true);
  });
  it("keeps the row while a chip in view is switched off", () => {
    expect(showMilestoneRow(chips("24.1"), ["24.1"])).toBe(true);
    expect(showMilestoneRow(chips(NO_MILESTONE), [NO_MILESTONE])).toBe(true);
    // Switched off elsewhere only: no reason to show it here.
    expect(showMilestoneRow(chips("24.1"), ["v5.1.0"])).toBe(false);
  });
});

describe("soloToggles", () => {
  const chips = ["v5.1.0", "v5.2.0", NO_MILESTONE].map((key) => ({ key, label: key, count: 1 }));
  it("switches off every other chip in view", () => {
    expect(soloToggles(chips, [], "v5.2.0")).toEqual(["v5.1.0", NO_MILESTONE]);
  });
  it("switches the chosen one back on and leaves keys outside the view alone", () => {
    expect(soloToggles(chips, ["v5.2.0", "v9.0.0", NO_MILESTONE], "v5.2.0")).toEqual(["v5.2.0", "v5.1.0"]);
  });
  it("brings the others back when the chosen one is already the only one on", () => {
    expect(soloToggles(chips, ["v5.1.0", NO_MILESTONE, "v9.0.0"], "v5.2.0")).toEqual(["v5.1.0", NO_MILESTONE]);
  });
});
