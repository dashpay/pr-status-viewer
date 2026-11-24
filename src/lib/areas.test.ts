import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import { NO_AREA as STORE_NO_AREA } from "../state/store";
import type { PullRequest } from "../types";
import { NO_AREA, areaCounts, areaVisible, titleAreas } from "./areas";

const base = mockSnapshot(Date.parse("2026-11-22T12:00:00Z")).projects[0].prs[0];
const pr = (title: string, repo = "o/r"): PullRequest => ({ ...base, id: `${repo}#${title}`, repo, title });

describe("titleAreas", () => {
  it("reads the scope of a conventional title", () => {
    expect(titleAreas("fix(sdk): read document history")).toEqual(["sdk"]);
    expect(titleAreas("feat(platform-wallet)!: shielded balance")).toEqual(["platform-wallet"]);
    expect(titleAreas("  Fix(Drive-ABCI): apply block randomness")).toEqual(["drive-abci"]);
  });
  it("splits several scopes and drops repeats", () => {
    expect(titleAreas("fix(sdk, drive): one thing")).toEqual(["sdk", "drive"]);
    expect(titleAreas("fix(sdk,sdk , wallet): x")).toEqual(["sdk", "wallet"]);
  });
  it("has no area without a scope", () => {
    expect(titleAreas("docs: add DIP for shielded addresses")).toEqual([NO_AREA]);
    expect(titleAreas("The library to 90: books to the ceiling (plan 09)")).toEqual([NO_AREA]);
    expect(titleAreas("fix(): empty scope")).toEqual([NO_AREA]);
    expect(titleAreas("Merge branch (sdk): not a type prefix")).toEqual([NO_AREA]);
  });
  it("only reads the leading prefix", () => {
    expect(titleAreas("fix(sdk): handle f(x): case")).toEqual(["sdk"]);
  });
});

describe("areaCounts", () => {
  it("counts by area, sorted by name, with no-scope last", () => {
    const prs = [pr("fix(sdk): a"), pr("docs: b"), pr("feat(drive, sdk): c"), pr("chore(dashmate): d")];
    expect(areaCounts(prs).map((a) => [a.label, a.count])).toEqual([
      ["dashmate", 1],
      ["drive", 1],
      ["sdk", 2],
      ["Other", 1],
    ]);
  });
});

describe("areaVisible", () => {
  it("hides a pull request only when all its areas are off, in its own project", () => {
    const hidden = { "o/r": ["sdk"] };
    expect(areaVisible(pr("fix(sdk): a"), hidden)).toBe(false);
    expect(areaVisible(pr("fix(sdk, drive): a"), hidden)).toBe(true);
    expect(areaVisible(pr("fix(drive): a"), hidden)).toBe(true);
    expect(areaVisible(pr("fix(sdk): a", "other/repo"), hidden)).toBe(true);
    expect(areaVisible(pr("docs: a"), { "o/r": [NO_AREA] })).toBe(false);
    expect(areaVisible(pr("fix(sdk): a"), {})).toBe(true);
  });
  it("uses the same no-area key as the store", () => {
    expect(NO_AREA).toBe(STORE_NO_AREA);
  });
});
