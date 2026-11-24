import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import type { PullRequest } from "../types";
import { stackedLabel, stackedTitle, stackPrs } from "./stack";

const base = mockSnapshot(Date.parse("2026-11-22T12:00:00Z")).projects[0].prs[0];

function pr(number: number, stackedOn: number | null = null, repo = "o/r"): PullRequest {
  return { ...base, id: `${repo}#${number}`, repo, number, title: `feat: #${number}`, stackedOn };
}

/** "6116 6132:1 6133:2": each row's number, with its depth when nested. */
function shape(list: readonly PullRequest[]): string {
  const { prs, places } = stackPrs(list);
  return prs
    .map((p) => {
      const d = places.get(p.id)?.depth;
      return d ? `${p.number}:${d}` : String(p.number);
    })
    .join(" ");
}

describe("stackPrs", () => {
  it("leaves a list without stacks as it is", () => {
    expect(shape([pr(3), pr(1), pr(2)])).toBe("3 1 2");
  });

  it("lists a chain of three under each other, one step deeper each", () => {
    // dashpay/platform: #6133 is stacked on #6132, which is stacked on #6116.
    // Sorted with the children first, as a recent push would put them.
    const list = [pr(6133, 6132), pr(7000), pr(6132, 6116), pr(6116), pr(6001)];
    expect(shape(list)).toBe("7000 6116 6132:1 6133:2 6001");
    const { places } = stackPrs(list);
    expect(places.get("o/r#6133")?.parent.number).toBe(6132);
    expect(places.get("o/r#6132")?.parent.number).toBe(6116);
    expect(places.has("o/r#6116")).toBe(false);
  });

  it("keeps two children of one parent in their sorted order, each with its own descendants", () => {
    // #5750 on #5749; #5751 and #5753 both on #5750; #5760 on #5751.
    const list = [pr(5753, 5750), pr(5749), pr(5751, 5750), pr(5800), pr(5750, 5749), pr(5760, 5751)];
    expect(shape(list)).toBe("5749 5750:1 5753:2 5751:2 5760:3 5800");
  });

  it("leaves a PR whose parent is not listed at its own place, at the left edge", () => {
    expect(shape([pr(2), pr(6133, 6132), pr(1)])).toBe("2 6133 1");
    expect(stackPrs([pr(6133, 6132)]).places.size).toBe(0);
  });

  it("only stacks on a parent in the same project", () => {
    expect(shape([pr(5, 4, "o/a"), pr(4, null, "o/b")])).toBe("5 4");
    // o/a#5 moves under o/a#4, not under o/b#4.
    expect(shape([pr(5, 4, "o/a"), pr(4, null, "o/b"), pr(4, null, "o/a")])).toBe("4 4 5:1");
    expect(stackPrs([pr(5, 4, "o/a"), pr(4, null, "o/b"), pr(4, null, "o/a")]).places.get("o/a#5")?.parent.id).toBe(
      "o/a#4",
    );
  });

  it("does not loop on a cycle, cutting it at the member sorted first", () => {
    // #2 on #1, #1 on #3, #3 on #2. #2 sorts first, so the cycle is cut there:
    // #2 leads, #3 (on #2) follows, then #1 (on #3).
    const list = [pr(2, 1), pr(9), pr(1, 3), pr(3, 2)];
    const { prs, places } = stackPrs(list);
    expect(prs).toHaveLength(4);
    expect(new Set(prs.map((p) => p.number)).size).toBe(4);
    // #2 (the cycle's first row) is cut loose and leads its stack at its own place.
    expect(places.has("o/r#2")).toBe(false);
    expect(shape(list)).toBe("2 3:1 1:2 9");
    // A PR stacked on itself is simply not stacked.
    expect(shape([pr(1, 1), pr(2)])).toBe("1 2");
  });

  it("treats a missing stackedOn (an older captured snapshot) as not stacked", () => {
    const old = { ...pr(5, 4) } as Partial<PullRequest>;
    delete old.stackedOn;
    expect(shape([old as PullRequest, pr(4)])).toBe("5 4");
  });
});

describe("stacked labels", () => {
  it("names the parent by number, and by title in the tooltip", () => {
    const parent = { number: 6132, title: "feat(sdk): token pricing" };
    expect(stackedLabel(parent)).toBe("Stacked on #6132");
    expect(stackedTitle(parent)).toBe("Stacked on #6132: feat(sdk): token pricing");
  });
});
