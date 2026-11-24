import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import { allPrs, prsForView } from "./select";
import { groupPrs, primaryArea, splitBySubcrate, statusSummary } from "./group";
import { sortPrs } from "./sort";

const NOW = Date.parse("2026-11-22T12:00:00Z");
const snap = mockSnapshot(NOW);
const numbers = (prs: { number: number }[]) => prs.map((p) => p.number);

describe("sortPrs", () => {
  it("puts PRs with a finish time first, soonest first, then the rest by update", () => {
    const sorted = sortPrs(allPrs(snap), "finishesFirst");
    // 6033 finishes in ~20 min; 1161 and 1213 in ~47 min (same time, 1213 has
    // the same update time so they fall back to id order); 7887 in ~58 min.
    expect(numbers(sorted).slice(0, 4)).toEqual([6033, 1161, 1213, 7887]);
    expect(numbers(sorted).slice(4, 7)).toEqual([6038, 6022, 1219]);
  });
  it("orders finished reviews by the last review, not by last update", () => {
    const at = (min: number) => new Date(NOW - min * 60_000).toISOString();
    const base = snap.projects[0].prs[0];
    const verdict = (number: number, reviewMinAgo: number, updatedMinAgo: number) => ({
      ...base,
      id: `o/r#${number}`,
      number,
      updatedAt: at(updatedMinAgo),
      lastReviewAt: at(reviewMinAgo),
      bot: { state: "approved" as const, at: at(reviewMinAgo) },
    });
    // #1 got its last review 4 h ago but was commented on a minute ago.
    const none = { ...base, id: "o/r#3", number: 3, lastReviewAt: null, bot: { state: "none" as const } };
    const prs = [verdict(1, 240, 1), verdict(2, 26, 30), none];
    expect(numbers(sortPrs(prs, "finishesFirst"))).toEqual([2, 1, 3]);
    expect(numbers(sortPrs(prs.slice(0, 2), "updated"))).toEqual([1, 2]);
  });
  it("sorts by most recently updated", () => {
    const sorted = sortPrs(allPrs(snap), "updated");
    expect(numbers(sorted).slice(0, 3)).toEqual([6033, 7887, 6038]);
  });
});

describe("statusSummary", () => {
  it("counts non-zero statuses in status order", () => {
    const platform = snap.projects[0].prs;
    expect(statusSummary(platform)).toBe(
      "1 reviewing · 4 changes requested · 1 commented · 2 approved · 2 draft",
    );
    expect(statusSummary(snap.projects[2].prs)).toBe("2 queued · 1 changes requested · 1 draft");
  });
});

describe("groupPrs", () => {
  it("groups by project in snapshot order, leaving out empty repos", () => {
    const groups = groupPrs(prsForView(snap, { kind: "following" }), "project", "finishesFirst", snap);
    expect(groups.map((g) => g.title)).toEqual(["dashpay/dash-evo-tool", "dashpay/dash"]);
    expect(groups[0].meta).toBe("2 queued · 1 changes requested · 1 draft");
  });
  it("carries a project's fetch error", () => {
    const broken = { ...snap, projects: snap.projects.map((p, i) => (i === 1 ? { ...p, error: "Not found" } : p)) };
    const groups = groupPrs(prsForView(broken, { kind: "mine" }), "project", "updated", broken);
    expect(groups.find((g) => g.title === "dashpay/dips")?.error).toBe("Not found");
  });
  it("groups by status in a fixed order with counts", () => {
    const groups = groupPrs(allPrs(snap), "status", "finishesFirst", snap);
    expect(groups.map((g) => `${g.title} ${g.meta}`)).toEqual([
      "Reviewing 1",
      "Queued 3",
      "Changes requested 5",
      "Comments only 1",
      "Approved 2",
      "Draft 3",
      "No bot review 1",
    ]);
    expect(numbers(groups[1].prs)).toEqual([1161, 1213, 7887]);
  });
  it("puts an unrecognised bot state under No bot review", () => {
    const pr = { ...allPrs(snap)[0], bot: { state: "other" as const, title: "Cancelled" } };
    expect(groupPrs([pr], "status", "updated", snap)[0].title).toBe("No bot review");
  });
});

describe("primaryArea", () => {
  const pr = (title: string) => ({ ...allPrs(snap)[0], repo: "o/r", title });
  it("is the title's scope, or Other's key when it has none", () => {
    expect(primaryArea(pr("fix(SDK): x"))).toBe("sdk");
    expect(primaryArea(pr("feat!: x"))).toBe("");
    expect(primaryArea(pr("feat(): x"))).toBe("");
  });
  it("is the first area written when a title names several", () => {
    expect(primaryArea(pr("fix(sdk, drive-abci): x"))).toBe("sdk");
  });
  it("skips areas switched off in the PR's project, so the PR lands under one that is on", () => {
    expect(primaryArea(pr("fix(sdk, drive-abci): x"), { "o/r": ["sdk"] })).toBe("drive-abci");
    // Another project's switches do not count.
    expect(primaryArea(pr("fix(sdk, drive-abci): x"), { "o/other": ["sdk"] })).toBe("sdk");
  });
});

describe("splitBySubcrate", () => {
  const base = allPrs(snap)[0];
  const pr = (number: number, title: string) => ({ ...base, id: `o/r#${number}`, number, repo: "o/r", title });
  it("keeps the row order inside each sub-group and orders sub-groups by their first row", () => {
    const sorted = [pr(1, "fix(drive): a"), pr(2, "fix: b"), pr(3, "fix(sdk): c"), pr(4, "fix(drive): d"), pr(5, "x")];
    const subs = splitBySubcrate("o/r", sorted);
    expect(subs.map((s) => [s.title, numbers(s.prs)])).toEqual([
      ["drive", [1, 4]],
      ["Other", [2, 5]],
      ["sdk", [3]],
    ]);
    expect(subs.map((s) => s.key)).toEqual(["o/r::drive", "o/r::", "o/r::sdk"]);
    expect(subs[1].area).toBe("");
  });
  it("lists a PR naming several areas once", () => {
    const subs = splitBySubcrate("o/r", [pr(1, "fix(sdk, drive): a"), pr(2, "fix(drive): b")]);
    expect(subs.map((s) => [s.title, numbers(s.prs)])).toEqual([
      ["sdk", [1]],
      ["drive", [2]],
    ]);
  });
});

describe("groupPrs by subcrate", () => {
  const mine = prsForView(snap, { kind: "mine" });
  const split = (sortBy: "finishesFirst" | "updated", hiddenAreas = {}) =>
    groupPrs(mine, "project", sortBy, snap, { bySubcrate: true, hiddenAreas });
  it("splits each project group, Other included, and shows one sub-group even when it is the only one", () => {
    const groups = split("finishesFirst");
    expect(groups.map((g) => g.title)).toEqual(["dashpay/platform", "dashpay/dips"]);
    expect(groups[0].subgroups?.map((s) => `${s.title} ${s.prs.length}`)).toEqual([
      "swift-sdk 1",
      "platform 2",
      "sdk 3",
      "kotlin-example-app 1",
      "platform-wallet 1",
      "dashmate 1",
      "Other 1",
    ]);
    expect(groups[1].subgroups?.map((s) => s.title)).toEqual(["Other"]);
  });
  it("keeps the project header's summary and lists the rows sub-group by sub-group", () => {
    const plain = groupPrs(mine, "project", "finishesFirst", snap);
    const groups = split("finishesFirst");
    expect(groups[0].meta).toBe(plain[0].meta);
    expect(groups[0].prs).toEqual(groups[0].subgroups?.flatMap((s) => s.prs));
    expect([...numbers(groups[0].prs)].sort()).toEqual([...numbers(plain[0].prs)].sort());
    // The sort still decides what is on top: the first row overall leads.
    expect(groups[0].prs[0]).toBe(plain[0].prs[0]);
  });
  it("orders sub-groups by the sort", () => {
    const first = (sortBy: "finishesFirst" | "updated") => split(sortBy)[0].subgroups?.[0].prs[0].number;
    expect(first("finishesFirst")).toBe(groupPrs(mine, "project", "finishesFirst", snap)[0].prs[0].number);
    expect(first("updated")).toBe(groupPrs(mine, "project", "updated", snap)[0].prs[0].number);
  });
  it("is off unless asked, and never splits status groups", () => {
    expect(groupPrs(mine, "project", "updated", snap).every((g) => g.subgroups === null)).toBe(true);
    expect(
      groupPrs(mine, "status", "updated", snap, { bySubcrate: true }).every((g) => g.subgroups === null),
    ).toBe(true);
  });
  it("places a PR naming several areas under the first one switched on", () => {
    const multi = mine.find((p) => p.title.startsWith("fix(sdk, drive-abci)"));
    expect(multi).toBeDefined();
    const where = (hidden: Record<string, string[]>) =>
      split("finishesFirst", hidden)[0].subgroups?.find((s) => s.prs.includes(multi!))?.title;
    expect(where({})).toBe("sdk");
    expect(where({ "dashpay/platform": ["sdk"] })).toBe("drive-abci");
  });
});

describe("groupPrs with stacked PRs", () => {
  // The mock stacks #5750 on #5749 and #5967 on #5750 (all dashpay/platform).
  const mine = prsForView(snap, { kind: "mine" });
  const depth = (g: { stacked: ReadonlyMap<string, { depth: number }> }, n: number) =>
    g.stacked.get(`dashpay/platform#${n}`)?.depth ?? 0;

  it("lists a stack under its parent in a project group, deeper levels one step further", () => {
    for (const sortBy of ["finishesFirst", "updated"] as const) {
      const platform = groupPrs(mine, "project", sortBy, snap)[0];
      const ids = numbers(platform.prs);
      const at = ids.indexOf(5749);
      // The point of the mock's stack: #5750 and #5967 follow #5749 directly,
      // although the sort alone puts #5750 above it.
      expect(ids.slice(at, at + 3)).toEqual([5749, 5750, 5967]);
      expect([depth(platform, 5749), depth(platform, 5750), depth(platform, 5967)]).toEqual([0, 1, 2]);
      expect(platform.stacked.get("dashpay/platform#5967")?.parent.number).toBe(5750);
      expect(platform.stacked.size).toBe(2);
      // Nothing is lost or repeated.
      expect([...ids].sort()).toEqual(numbers(sortPrs(platform.prs, sortBy)).sort());
    }
    expect(numbers(sortPrs(snap.projects[0].prs, "finishesFirst")).indexOf(5750)).toBeLessThan(
      numbers(sortPrs(snap.projects[0].prs, "finishesFirst")).indexOf(5749),
    );
  });

  it("does not nest across status groups: each PR stays at its own place there", () => {
    // #5749 changes requested, #5750 approved, #5967 draft: three groups.
    const groups = groupPrs(allPrs(snap), "status", "finishesFirst", snap);
    for (const g of groups) expect(g.stacked.size).toBe(0);
    expect(numbers(groups.find((g) => g.title === "Approved")!.prs)).toContain(5750);
  });

  it("nests inside a status group when parent and child share it", () => {
    const all = allPrs(snap).map((p) => (p.number === 5750 ? { ...p, bot: snap.projects[0].prs.find((q) => q.number === 5749)!.bot } : p));
    const changes = groupPrs(all, "status", "finishesFirst", snap).find((g) => g.title === "Changes requested")!;
    const ids = numbers(changes.prs);
    expect(ids.indexOf(5750)).toBe(ids.indexOf(5749) + 1);
    expect(depth(changes, 5750)).toBe(1);
  });

  it("nests inside a subcrate sub-group, not across sub-groups", () => {
    const split = (prs: typeof mine) => groupPrs(prs, "project", "finishesFirst", snap, { bySubcrate: true })[0];
    // In the mock the three sit in different sub-groups (Other, platform, sdk): no nesting.
    const apart = split(mine);
    expect(apart.stacked.size).toBe(0);
    expect(apart.prs).toEqual(apart.subgroups?.flatMap((s) => s.prs));
    // Give #5749 and #5967 the platform scope too: one sub-group, one stack.
    const retitled = mine.map((p) =>
      p.number === 5749 || p.number === 5967 ? { ...p, title: p.title.replace(/^feat(\([^)]*\))?/, "feat(platform)") } : p,
    );
    const together = split(retitled);
    const sub = together.subgroups!.find((s) => s.title === "platform")!;
    const ids = numbers(sub.prs);
    const at = ids.indexOf(5749);
    expect(ids.slice(at, at + 3)).toEqual([5749, 5750, 5967]);
    expect([depth(together, 5750), depth(together, 5967)]).toEqual([1, 2]);
    expect(together.prs).toEqual(together.subgroups?.flatMap((s) => s.prs));
  });
});
