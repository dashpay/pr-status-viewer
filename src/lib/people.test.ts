import { describe, expect, it } from "vitest";
import { contributorTitle, filterKnown, openPullRequests, possessive, typedLogin } from "./people";

describe("possessive and contributorTitle", () => {
  it("adds 's, or only an apostrophe after a final s", () => {
    expect(possessive("shumkov")).toBe("shumkov's");
    expect(possessive("tidepools")).toBe("tidepools'");
    expect(possessive("ANTS")).toBe("ANTS'");
    expect(contributorTitle("OrbitWren")).toBe("OrbitWren's pull requests");
    expect(contributorTitle("tidepools")).toBe("tidepools' pull requests");
  });
});

describe("openPullRequests", () => {
  it("counts in words", () => {
    expect(openPullRequests(0)).toBe("0 open pull requests");
    expect(openPullRequests(1)).toBe("1 open pull request");
    expect(openPullRequests(3)).toBe("3 open pull requests");
  });
});

describe("typedLogin", () => {
  it("takes a login from what people type or paste", () => {
    expect(typedLogin("  shu ")).toBe("shu");
    expect(typedLogin("@OrbitWren")).toBe("OrbitWren");
    expect(typedLogin("https://github.com/kestrel-dev")).toBe("kestrel-dev");
    expect(typedLogin("github.com/kestrel-dev/")).toBe("kestrel-dev");
    expect(typedLogin("https://www.github.com/tidepools?tab=repositories")).toBe("tidepools");
    expect(typedLogin("")).toBe("");
  });
});

describe("filterKnown", () => {
  const people = [
    { login: "OrbitWren", avatarUrl: null, openPrs: 3 },
    { login: "kestrel-dev", avatarUrl: null, openPrs: 2 },
    { login: "LumenAsh", avatarUrl: null, openPrs: 1 },
  ];

  it("keeps everyone for an empty field", () => {
    expect(filterKnown(people, "  ")).toEqual(people);
  });

  it("narrows by login, any casing, in the list's order", () => {
    expect(filterKnown(people, "REN").map((p) => p.login)).toEqual(["OrbitWren"]);
    expect(filterKnown(people, "@e").map((p) => p.login)).toEqual(["OrbitWren", "kestrel-dev", "LumenAsh"]);
    expect(filterKnown(people, "github.com/lumen").map((p) => p.login)).toEqual(["LumenAsh"]);
    expect(filterKnown(people, "nobody")).toEqual([]);
  });
});
