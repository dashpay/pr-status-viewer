import { describe, expect, it } from "vitest";
import type { RunJob, RunPlatformOptions, RunSource, RunStep } from "../types";
import {
  defaultDevice,
  defaultSource,
  deviceLabel,
  durationText,
  headerStatusText,
  jobElapsedText,
  jobHeading,
  platformsFor,
  prNumberOf,
  rowMarkerText,
  shortSha,
  shortenPath,
  stepDurationText,
} from "./run";

const T0 = Date.parse("2026-11-22T10:00:00Z");
const iso = (s: number) => new Date(T0 + s * 1000).toISOString();

const head: RunSource = { kind: "prHead", path: "/app/checkout", branch: null, sha: "abc1234ffff", atPrHead: true, dirty: false };
const tree: RunSource = { kind: "worktree", path: "/Users/me/wt", branch: "fix/x", sha: "def5678", atPrHead: false, dirty: true };

const step = (patch: Partial<RunStep>): RunStep => ({
  key: "buildSdk",
  label: "Build the SDK",
  status: "pending",
  startedAt: null,
  finishedAt: null,
  ...patch,
});

const job = (patch: Partial<RunJob> = {}): RunJob => ({
  jobId: 1,
  prId: "dashpay/platform#5736",
  platform: "ios",
  deviceName: "iPhone 17",
  sourceKind: "prHead",
  sha: "abc1234",
  state: "running",
  steps: [step({ key: "checkout", label: "Check out the code", status: "done", startedAt: iso(0), finishedAt: iso(5) }), step({ status: "running", startedAt: iso(5) })],
  startedAt: iso(0),
  finishedAt: null,
  error: null,
  logPath: null,
  ...patch,
});

describe("platformsFor", () => {
  const targets = [
    { repo: "a/b", clonePath: "/x", cloneIsSet: false, platforms: ["android" as const, "ios" as const] },
    { repo: "c/d", clonePath: null, cloneIsSet: false, platforms: [] },
  ];
  it("lists the offered platforms, iOS first", () => {
    expect(platformsFor(targets, "a/b")).toEqual(["ios", "android"]);
  });
  it("offers nothing for a project without apps or not listed", () => {
    expect(platformsFor(targets, "c/d")).toEqual([]);
    expect(platformsFor(targets, "e/f")).toEqual([]);
  });
});

describe("small labels", () => {
  it("reads PR numbers and short commits", () => {
    expect(prNumberOf("dashpay/platform#5736")).toBe(5736);
    expect(shortSha("abc1234ffff")).toBe("abc1234");
  });

  it("shortens paths: home as ~, middle elided when long", () => {
    expect(shortenPath("/Users/me/src/platform")).toBe("~/src/platform");
    expect(shortenPath("/home/me/src/platform")).toBe("~/src/platform");
    expect(shortenPath("/opt/src")).toBe("/opt/src");
    const long = shortenPath("/Users/me/.t3/worktrees/platform/some-very-long-worktree-name-for-a-branch/x", 40);
    expect(long.startsWith("~/…/")).toBe(true);
    expect(long.endsWith("/x")).toBe(true);
    expect(long.length).toBeLessThanOrEqual(60);
  });

  it("labels devices with their detail and whether they run", () => {
    expect(deviceLabel({ id: "1", name: "iPhone 17", detail: "iOS 26.1", kind: "simulator", running: true })).toBe(
      "iPhone 17 · iOS 26.1 · Running",
    );
    expect(deviceLabel({ id: "2", name: "pixel", detail: null, kind: "emulator", running: false })).toBe("pixel");
  });
});

describe("dialog defaults", () => {
  it("picks the remembered source kind when offered, else the first source", () => {
    expect(defaultSource([head, tree], { sourceKind: "worktree", devices: {} })).toBe(tree);
    expect(defaultSource([head], { sourceKind: "worktree", devices: {} })).toBe(head);
    expect(defaultSource([head, tree], { sourceKind: null, devices: {} })).toBe(head);
    expect(defaultSource([], { sourceKind: null, devices: {} })).toBeNull();
  });

  it("picks the remembered device when still listed, else the first", () => {
    const opts: RunPlatformOptions = {
      platform: "ios",
      unavailable: null,
      devices: [
        { id: "a", name: "A", detail: null, kind: "simulator", running: true },
        { id: "b", name: "B", detail: null, kind: "simulator", running: false },
      ],
    };
    expect(defaultDevice(opts, { sourceKind: null, devices: { ios: "b" } })?.id).toBe("b");
    expect(defaultDevice(opts, { sourceKind: null, devices: { ios: "gone" } })?.id).toBe("a");
    expect(defaultDevice(opts, { sourceKind: null, devices: { android: "b" } })?.id).toBe("a");
    expect(defaultDevice({ ...opts, devices: [] }, { sourceKind: null, devices: {} })).toBeNull();
    expect(defaultDevice(undefined, { sourceKind: null, devices: {} })).toBeNull();
  });
});

describe("job text", () => {
  it("names the job", () => {
    expect(jobHeading(job())).toBe("iOS · #5736 · iPhone 17");
  });

  it("says what the header shows in each state", () => {
    expect(headerStatusText(job())).toBe("Running on iOS · Build the SDK");
    expect(headerStatusText(job({ steps: [] }))).toBe("Running on iOS");
    expect(headerStatusText(job({ state: "succeeded" }))).toBe("Opened on iPhone 17");
    expect(headerStatusText(job({ state: "failed", platform: "android" }))).toBe("Android run failed");
    expect(headerStatusText(job({ state: "cancelled" }))).toBe("iOS run cancelled");
  });

  it("marks only the running PR's row, only while running", () => {
    expect(rowMarkerText(job(), "dashpay/platform#5736")).toBe("Running on iOS");
    expect(rowMarkerText(job(), "dashpay/platform#1")).toBeNull();
    expect(rowMarkerText(job({ state: "succeeded" }), "dashpay/platform#5736")).toBeNull();
    expect(rowMarkerText(null, "dashpay/platform#5736")).toBeNull();
  });

  it("measures the job and its steps", () => {
    expect(durationText(65_000)).toBe("1 min 5 s");
    expect(jobElapsedText(job(), T0 + 42_000)).toBe("42 s");
    expect(jobElapsedText(job({ finishedAt: iso(90) }), T0 + 999_000)).toBe("1 min 30 s");
    const [done, running] = job().steps;
    expect(stepDurationText(done, T0 + 100_000)).toBe("5 s");
    expect(stepDurationText(running, T0 + 17_000)).toBe("12 s");
    expect(stepDurationText(step({}), T0)).toBeNull();
    expect(stepDurationText(step({ status: "done", startedAt: iso(0) }), T0)).toBeNull();
  });
});
