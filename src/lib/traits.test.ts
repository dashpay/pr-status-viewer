import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import type { PrTrait, PullRequest } from "../types";
import { traitAction, traitLabel, traitSlots, traitsOf } from "./traits";

const template = mockSnapshot().projects[0].prs[0];
const withTraits = (traits: unknown): PullRequest => ({ ...template, traits: traits as PrTrait[] });
const grove: PrTrait = { kind: "grovedbChange", summary: "5 nodes changed", links: [{ label: "Structure viewer", url: "https://g" }] };
const contract: PrTrait = { kind: "contractChange", summary: "", links: [] };

describe("traitsOf", () => {
  it("treats a missing or null field as no traits", () => {
    const pr = { ...template } as Partial<PullRequest>;
    delete pr.traits;
    expect(traitsOf(pr as PullRequest)).toEqual([]);
    expect(traitsOf(withTraits(null))).toEqual([]);
  });

  it("puts GroveDB first and drops kinds it does not know", () => {
    const unknown = { kind: "somethingNew", summary: "x", links: [] };
    expect(traitsOf(withTraits([contract, unknown, grove])).map((t) => t.kind)).toEqual(["grovedbChange", "contractChange"]);
  });
});

describe("traitSlots", () => {
  it("is the most traits any pull request has", () => {
    expect(traitSlots([])).toBe(0);
    expect(traitSlots([withTraits([]), withTraits([grove, contract]), withTraits([grove])])).toBe(2);
  });
});

describe("traitLabel", () => {
  it("names the kind, the summary and what a click does", () => {
    expect(traitLabel(grove)).toBe("GroveDB change: 5 nodes changed. Open the structure viewer");
    expect(traitLabel(contract)).toBe("Contract change");
    const two = { ...contract, summary: "2 contracts changed", links: [grove.links[0], grove.links[0]] };
    expect(traitLabel(two)).toBe("Contract change: 2 contracts changed. Choose one of 2 links to open");
  });

  it("does nothing for links without a URL", () => {
    expect(traitAction({ ...grove, links: [{ label: "Broken", url: "" }] })).toBeNull();
  });
});
