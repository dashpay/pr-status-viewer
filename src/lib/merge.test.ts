import { describe, expect, it } from "vitest";
import { mockSnapshot } from "../mock/snapshot";
import type { PullRequest, T3Thread } from "../types";
import {
  NO_PERMISSION_REASON,
  commentsText,
  mergeWarnings,
  showMergeButton,
  sortT3Threads,
  t3RowTag,
  t3StateText,
  t3Working,
} from "./merge";

const NOW = Date.parse("2026-11-22T12:00:00Z");
const base = mockSnapshot(NOW).projects[0].prs[0];
const make = (patch: Partial<PullRequest>): PullRequest => ({ ...base, ...patch });

describe("commentsText", () => {
  it("reads all resolved, singular and plural", () => {
    expect(commentsText(0)).toBe("All resolved");
    expect(commentsText(1)).toBe("1 unresolved");
    expect(commentsText(3)).toBe("3 unresolved");
  });
});

describe("showMergeButton", () => {
  it("hides the button only for the no-permission reason", () => {
    expect(showMergeButton(make({ merge: { ready: true, reason: null } }))).toBe(true);
    expect(showMergeButton(make({ merge: { ready: false, reason: "3 unresolved comments" } }))).toBe(true);
    expect(showMergeButton(make({ merge: { ready: false, reason: NO_PERMISSION_REASON } }))).toBe(false);
  });
});

describe("mergeWarnings", () => {
  it("is empty for passing CI, an approving bot and no change requests", () => {
    expect(mergeWarnings(make({ ci: "passing", bot: { state: "approved", at: null }, humans: [] }))).toEqual([]);
  });

  it("names failing or running CI, the bot's state and humans who requested changes", () => {
    expect(
      mergeWarnings(
        make({
          ci: "failing",
          bot: { state: "changesRequested", at: null },
          humans: [
            { login: "UdjinM6", avatarUrl: null, state: "changesRequested" },
            { login: "knst", avatarUrl: null, state: "approved" },
          ],
        }),
      ),
    ).toEqual(["CI is failing.", "PastaClaw requested changes.", "UdjinM6 requested changes."]);
    expect(mergeWarnings(make({ ci: "pending", bot: { state: "none" }, humans: [] }))).toEqual([
      "CI is still running.",
      "PastaClaw has not reviewed this pull request.",
    ]);
  });
});

describe("T3 threads", () => {
  const idle = {
    threadId: "a",
    title: "Idle one",
    state: "idle" as const,
    lastActiveAt: "2026-11-22T10:00:00Z",
    watching: false,
  };
  const working = { threadId: "b", title: "Busy one", state: "working" as const, lastActiveAt: null, watching: false };
  const waiting = {
    threadId: "c",
    title: "Waiting one",
    state: "waiting" as const,
    lastActiveAt: "2026-11-22T11:55:00Z",
    watching: true,
  };

  it("says whether any thread is working", () => {
    expect(t3Working(make({ t3Threads: [] }))).toBe(false);
    expect(t3Working(make({ t3Threads: [idle] }))).toBe(false);
    expect(t3Working(make({ t3Threads: [waiting] }))).toBe(false);
    expect(t3Working(make({ t3Threads: [idle, working] }))).toBe(true);
  });

  it("puts working threads first, then waiting, then idle", () => {
    expect(sortT3Threads([idle, working]).map((t) => t.threadId)).toEqual(["b", "a"]);
    expect(sortT3Threads([idle, waiting, working]).map((t) => t.threadId)).toEqual(["b", "c", "a"]);
    const idle2 = { ...idle, threadId: "d" };
    expect(sortT3Threads([idle2, idle, waiting]).map((t) => t.threadId)).toEqual(["c", "d", "a"]);
  });

  it("describes each state", () => {
    expect(t3StateText(working, NOW)).toBe("working now");
    expect(t3StateText({ ...working, watching: true }, NOW)).toBe("working now");
    expect(t3StateText(idle, NOW)).toBe("idle, last active 2 h ago");
    expect(t3StateText({ ...idle, lastActiveAt: null }, NOW)).toBe("idle");
    expect(t3StateText({ ...idle, watching: true }, NOW)).toBe("idle, last active 2 h ago · watching this PR");
    expect(t3StateText(waiting, NOW)).toBe("waiting, last active 5 min ago · watching this PR");
    expect(t3StateText({ ...waiting, watching: false }, NOW)).toBe("waiting, last active 5 min ago");
    expect(t3StateText({ ...waiting, lastActiveAt: null }, NOW)).toBe("waiting · watching this PR");
    expect(t3StateText({ ...waiting, lastActiveAt: null, watching: false }, NOW)).toBe("waiting");
  });

  it("reads a missing watching flag as false", () => {
    const old = { threadId: "e", title: "Old backend", state: "idle", lastActiveAt: null } as unknown as T3Thread;
    expect(t3StateText(old, NOW)).toBe("idle");
  });

  it("tags a row working, else waiting, else not at all", () => {
    expect(t3RowTag(make({ t3Threads: [] }))).toBeNull();
    expect(t3RowTag(make({ t3Threads: [idle, { ...idle, watching: true }] }))).toBeNull();
    expect(t3RowTag(make({ t3Threads: [waiting, working] }))).toEqual({
      state: "working",
      label: "T3 working",
      title: "An agent is working on this pull request in T3 Code",
    });
    expect(t3RowTag(make({ t3Threads: [idle, waiting] }))).toEqual({
      state: "waiting",
      label: "T3 waiting",
      title: "A T3 Code thread is watching this pull request and resumes when it changes",
    });
    expect(t3RowTag(make({ t3Threads: [{ ...waiting, watching: false }] }))?.title).toBe(
      "A T3 Code thread is waiting on something it started and resumes on its own",
    );
  });
});
