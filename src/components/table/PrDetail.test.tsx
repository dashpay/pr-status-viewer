import { describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { mockSnapshot } from "../../mock/snapshot";
import { createAppStore, StoreProvider } from "../../state/store";
import type { Api } from "../../api";
import type { MergeCheck, PullRequest, Snapshot, T3Thread } from "../../types";
import { NO_PERMISSION_REASON } from "../../lib/merge";
import { PrTable } from "./PrTable";

const HEAD = "aaaa111122223333444455556666777788889999";
const FRESH_HEAD = "ffff111122223333444455556666777788889999";

/** One project, every PR mine and plain unless patched. */
function snapshotWith(patches: Partial<PullRequest>[], extra: Partial<Snapshot> = {}): Snapshot {
  const base = mockSnapshot();
  const template = base.projects[0].prs[0];
  const prs: PullRequest[] = patches.map((patch, i) => {
    const n = i + 1;
    return {
      ...template,
      id: `acme/app#${n}`,
      repo: "acme/app",
      number: n,
      title: `PR ${n}`,
      url: `https://github.com/acme/app/pull/${n}`,
      headSha: HEAD,
      baseBranch: "main",
      ci: "passing",
      bot: { state: "approved", at: null },
      humans: [],
      reviewDecision: null,
      nextStep: null,
      unresolvedThreads: 0,
      merge: { ready: true, reason: null },
      selfReview: { state: "unavailable", postedAt: null },
      t3Threads: [],
      ...patch,
    };
  });
  return { ...base, t3Available: true, projects: [{ repo: "acme/app", openCount: prs.length, prs, error: null }], ...extra };
}

async function setup(snapshot: Snapshot, overrides: Partial<Api> = {}) {
  const api: Api = { ...createMockApi({ latencyMs: 0, snapshot }), ...overrides };
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
    </StoreProvider>,
  );
  return { store, api };
}

const row = (id: string) => document.querySelector<HTMLElement>(`[data-pr-row="${id}"]`);

async function openDetail(user: ReturnType<typeof userEvent.setup>, n: number) {
  await user.click(row(`acme/app#${n}`) as HTMLElement);
  return screen.getByRole("region", { name: `Details for #${n}` });
}

function fact(detail: HTMLElement, label: string): HTMLElement | null {
  const dt = within(detail).queryByText(label, { selector: "dt" });
  return dt?.closest(".pr-fact") ?? null;
}

/** A deferred promise, to hold a call open while the test looks at the UI. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const freshCheck = (patch: Partial<MergeCheck> = {}): MergeCheck => ({
  id: "acme/app#1",
  headSha: FRESH_HEAD,
  baseBranch: "main",
  commitTitle: "PR 1 (#1)",
  unresolvedThreads: 0,
  merge: { ready: true, reason: null },
  ...patch,
});

describe("Squash and merge button", () => {
  it("is enabled only when the PR is ready, and shows the reason when not", async () => {
    const user = userEvent.setup();
    await setup(
      snapshotWith([
        { merge: { ready: true, reason: null } },
        { unresolvedThreads: 3, merge: { ready: false, reason: "3 unresolved comments" } },
      ]),
    );
    let detail = await openDetail(user, 1);
    const ready = within(detail).getByRole("button", { name: "Squash and merge" });
    expect(ready).not.toHaveAttribute("aria-disabled");
    expect(ready).toBeEnabled();

    detail = await openDetail(user, 2);
    const blocked = within(detail).getByRole("button", { name: "Squash and merge" });
    expect(blocked).toHaveAttribute("aria-disabled", "true");
    // Still focusable, and the reason is visible text it points to.
    expect(blocked).not.toHaveAttribute("disabled");
    expect(within(detail).getByText("3 unresolved comments")).toBeVisible();
    expect(blocked).toHaveAccessibleDescription("3 unresolved comments");
    await user.click(blocked);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("is hidden when the viewer cannot merge in the repo", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{ merge: { ready: false, reason: NO_PERMISSION_REASON } }]));
    const detail = await openDetail(user, 1);
    expect(within(detail).queryByRole("button", { name: "Squash and merge" })).toBeNull();
    expect(within(detail).queryByText(NO_PERMISSION_REASON)).toBeNull();
  });

  it("is not triggered by keyboard shortcuts on the row", async () => {
    const user = userEvent.setup();
    const mergeCheck = vi.fn(async () => freshCheck());
    const squashMerge = vi.fn(async () => {});
    await setup(snapshotWith([{}]), { mergeCheck, squashMerge });
    await openDetail(user, 1);
    await user.keyboard("m");
    await user.keyboard("{Enter}");
    expect(mergeCheck).not.toHaveBeenCalled();
    expect(squashMerge).not.toHaveBeenCalled();
  });
});

describe("Comments fact", () => {
  it("reads all resolved, singular and plural, for every PR", async () => {
    const user = userEvent.setup();
    await setup(
      snapshotWith([
        { unresolvedThreads: 0 },
        { unresolvedThreads: 1, merge: { ready: false, reason: "1 unresolved comment" } },
        { unresolvedThreads: 3, merge: { ready: false, reason: NO_PERMISSION_REASON } },
      ]),
    );
    expect(fact(await openDetail(user, 1), "Comments")).toHaveTextContent("CommentsAll resolved");
    expect(fact(await openDetail(user, 2), "Comments")).toHaveTextContent("Comments1 unresolved");
    expect(fact(await openDetail(user, 3), "Comments")).toHaveTextContent("Comments3 unresolved");
  });
});

describe("Merge confirmation", () => {
  it("shows a loading state, then the fresh facts and warnings, with focus on Cancel", async () => {
    const user = userEvent.setup();
    const check = deferred<MergeCheck>();
    await setup(
      snapshotWith([
        {
          ci: "failing",
          bot: { state: "commented", at: null },
          humans: [{ login: "UdjinM6", avatarUrl: null, state: "changesRequested" }],
        },
      ]),
      { mergeCheck: () => check.promise },
    );
    const detail = await openDetail(user, 1);
    await user.click(within(detail).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog", { name: "Squash and merge #1?" });
    expect(dialog).toHaveAccessibleDescription("PR 1");
    expect(within(dialog).getByRole("status")).toHaveTextContent("Checking with GitHub…");
    expect(within(dialog).queryByRole("button", { name: "Squash and merge" })).toBeNull();

    await act(async () => check.resolve(freshCheck({ commitTitle: "PR 1 squashed (#1)" })));
    expect(within(dialog).queryByText("Checking with GitHub…")).toBeNull();
    expect(dialog).toHaveTextContent("Intomain");
    expect(dialog).toHaveTextContent("Commit titlePR 1 squashed (#1)");
    expect(dialog).toHaveTextContent("CommentsAll resolved");
    const warnings = within(dialog).getByRole("list", { name: "Warnings" });
    expect(within(warnings).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "!CI is failing.",
      "!PastaClaw commented but has not approved.",
      "!UdjinM6 requested changes.",
    ]);
    // Warnings inform; the merge is still offered.
    expect(within(dialog).getByRole("button", { name: "Squash and merge" })).toBeEnabled();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
  });

  it("shows CI still running", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{ ci: "pending" }]));
    await user.click(within(await openDetail(user, 1)).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog");
    expect(await within(dialog).findByText("CI is still running.")).toBeInTheDocument();
  });

  it("shows the reason and only Close when the fresh check says not ready", async () => {
    const user = userEvent.setup();
    const mergeCheck = vi.fn(async () =>
      freshCheck({ unresolvedThreads: 2, merge: { ready: false, reason: "2 unresolved comments" } }),
    );
    await setup(snapshotWith([{}]), { mergeCheck });
    await user.click(within(await openDetail(user, 1)).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog");
    expect(await within(dialog).findByText("2 unresolved comments", { exact: false })).toHaveTextContent(
      "Cannot merge yet: 2 unresolved comments",
    );
    expect(dialog).toHaveTextContent("Comments2 unresolved");
    expect(within(dialog).getAllByRole("button").map((b) => b.textContent)).toEqual(["Close"]);
    await user.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows a failed check with Retry", async () => {
    const user = userEvent.setup();
    const mergeCheck = vi
      .fn<Api["mergeCheck"]>()
      .mockRejectedValueOnce("GitHub could not be reached.")
      .mockResolvedValueOnce(freshCheck());
    await setup(snapshotWith([{}]), { mergeCheck });
    await user.click(within(await openDetail(user, 1)).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("GitHub could not be reached.");
    expect(within(dialog).queryByRole("button", { name: "Squash and merge" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Retry" }));
    expect(await within(dialog).findByRole("button", { name: "Squash and merge" })).toBeEnabled();
    expect(mergeCheck).toHaveBeenCalledTimes(2);
  });

  it("merges once with the fresh head, even on a double click, then closes and the PR leaves", async () => {
    const user = userEvent.setup();
    const api = createMockApi({ latencyMs: 0, snapshot: snapshotWith([{}, {}]) });
    const realMerge = api.squashMerge;
    const gate = deferred<void>();
    const squashMerge = vi.fn(async (id: string, _sha: string) => {
      await gate.promise;
      // The mock checks the head it knows; the fresh check's head stands for the real one.
      return realMerge(id, HEAD);
    });
    await setup(snapshotWith([{}, {}]), {
      getSnapshot: api.getSnapshot,
      onSnapshot: api.onSnapshot,
      mergeCheck: async () => freshCheck(),
      squashMerge,
    });
    await user.click(within(await openDetail(user, 1)).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog");
    const confirm = await within(dialog).findByRole("button", { name: "Squash and merge" });
    await user.dblClick(confirm);
    expect(squashMerge).toHaveBeenCalledTimes(1);
    expect(squashMerge).toHaveBeenCalledWith("acme/app#1", FRESH_HEAD);
    // In flight: progress shown, both buttons off, Esc does nothing.
    expect(within(dialog).getByRole("button", { name: "Merging…" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    await act(async () => gate.resolve());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(row("acme/app#1")).toBeNull();
    expect(row("acme/app#2")).not.toBeNull();
    expect(squashMerge).toHaveBeenCalledTimes(1);
  });

  it("shows a rejection and keeps the dialog open with the buttons back", async () => {
    const user = userEvent.setup();
    const squashMerge = vi.fn(async () => {
      throw "New commits were pushed. Review them, then try again.";
    });
    await setup(snapshotWith([{}]), { mergeCheck: async () => freshCheck(), squashMerge });
    await user.click(within(await openDetail(user, 1)).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog");
    await user.click(await within(dialog).findByRole("button", { name: "Squash and merge" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "New commits were pushed. Review them, then try again.",
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Squash and merge" })).toBeEnabled();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled();
    expect(row("acme/app#1")).not.toBeNull();
  });

  it("does not merge on Enter while Cancel has focus", async () => {
    const user = userEvent.setup();
    const squashMerge = vi.fn(async () => {});
    await setup(snapshotWith([{}]), { mergeCheck: async () => freshCheck(), squashMerge });
    await user.click(within(await openDetail(user, 1)).getByRole("button", { name: "Squash and merge" }));
    const dialog = screen.getByRole("dialog");
    await within(dialog).findByRole("button", { name: "Squash and merge" });
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(squashMerge).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("Findings and Last review", () => {
  const counts = (b: number[], s: number[], n: number[]): PullRequest["findingCounts"] => {
    const one = ([total, agreed, notAgreed, coderabbit = 0, awaiting = 0]: number[]) => ({
      total,
      agreed,
      notAgreed,
      awaiting,
      coderabbit,
    });
    return { blocking: one(b), suggestion: one(s), nitpick: one(n) };
  };

  it("spells the counts out in words and gives the exact time of the last review", async () => {
    const user = userEvent.setup();
    const lastReviewAt = new Date(Date.now() - 26 * 60_000 - 5_000).toISOString();
    await setup(
      snapshotWith([
        {
          bot: { state: "changesRequested", at: lastReviewAt },
          lastReviewAt,
          findingCounts: counts([2, 1, 1], [3, 0, 0], [4, 1, 0]),
        },
      ]),
    );
    const detail = await openDetail(user, 1);
    expect(fact(detail, "Findings")).toHaveTextContent(
      "FindingsBlockers 2 of 2 settled (1 not agreed) · Suggestions 0 of 3 settled · Nitpicks 1 of 4 settled",
    );
    const exact = new Date(lastReviewAt).toLocaleString(undefined, {
      weekday: "short",
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
    expect(fact(detail, "Last review")).toHaveTextContent(`Last review${exact} · 26 min ago`);
    // The verdict stays in words on the Reviewers line.
    expect(fact(detail, "Reviewers")).toHaveTextContent("PastaClaw requested changes");
  });

  it("says the counts are the last round's and names CodeRabbit's share", async () => {
    const user = userEvent.setup();
    const lastReviewAt = new Date(Date.now() - 3 * 3600_000).toISOString();
    await setup(
      snapshotWith([
        {
          bot: { state: "changesRequested", at: lastReviewAt },
          lastReviewAt,
          findingCounts: counts([2, 0, 1, 1], [3, 0, 0], [0, 0, 0]),
        },
        {
          bot: { state: "commented", at: lastReviewAt },
          lastReviewAt,
          findingCounts: counts([1, 1, 0], [2, 0, 0], [0, 0, 0]),
        },
      ]),
    );
    expect(fact(await openDetail(user, 1), "Findings")).toHaveTextContent(
      "FindingsBlockers 1 of 2 settled (1 not agreed), 1 from CodeRabbit · Suggestions 0 of 3 settled · Nitpicks none (last review round of each reviewer)",
    );
    const without = fact(await openDetail(user, 2), "Findings");
    expect(without).toHaveTextContent(
      "FindingsBlockers 1 of 1 settled · Suggestions 0 of 2 settled · Nitpicks none (last review round of each reviewer)",
    );
    expect(without).not.toHaveTextContent("CodeRabbit");
  });

  it("names the findings awaiting re-review", async () => {
    const user = userEvent.setup();
    const lastReviewAt = new Date(Date.now() - 3 * 3600_000).toISOString();
    await setup(
      snapshotWith([
        {
          bot: { state: "changesRequested", at: lastReviewAt },
          lastReviewAt,
          findingCounts: counts([5, 0, 0, 0, 5], [6, 2, 1, 0, 3], [0, 0, 0]),
        },
      ]),
    );
    expect(fact(await openDetail(user, 1), "Findings")).toHaveTextContent(
      "FindingsBlockers 0 of 5 settled, 5 awaiting re-review · Suggestions 3 of 6 settled (1 not agreed), 3 awaiting re-review · Nitpicks none (last review round of each reviewer)",
    );
  });

  it("says none for a clean review, and leaves both facts out when the bot never reviewed", async () => {
    const user = userEvent.setup();
    const zero = counts([0, 0, 0], [0, 0, 0], [0, 0, 0]);
    await setup(
      snapshotWith([
        { bot: { state: "approved", at: null }, lastReviewAt: new Date().toISOString(), findingCounts: zero },
        { bot: { state: "none" }, lastReviewAt: null, findingCounts: zero },
      ]),
    );
    expect(fact(await openDetail(user, 1), "Findings")).toHaveTextContent(
      "FindingsBlockers none · Suggestions none · Nitpicks none",
    );
    const never = await openDetail(user, 2);
    expect(fact(never, "Findings")).toBeNull();
    expect(fact(never, "Last review")).toBeNull();
  });
});

const ago = (hours: number) => new Date(Date.now() - hours * 3_600_000 - 60_000).toISOString();
const idle = (title: string, hoursAgo: number, watching = false): T3Thread => ({
  threadId: title,
  title,
  state: "idle",
  lastActiveAt: ago(hoursAgo),
  watching,
});
const waiting = (title: string, hoursAgo: number, watching = true): T3Thread => ({
  threadId: title,
  title,
  state: "waiting",
  lastActiveAt: ago(hoursAgo),
  watching,
});
const working = (title: string): T3Thread => ({ threadId: title, title, state: "working", lastActiveAt: null, watching: false });

describe("T3 Code indicator", () => {
  it("tags a row working while a thread works, else waiting while one waits", async () => {
    await setup(
      snapshotWith([
        { t3Threads: [idle("Old work", 2), waiting("Watch CI", 1), working("Fix CI")] },
        { t3Threads: [idle("Old work", 2, true)] },
        { t3Threads: [] },
        { t3Threads: [idle("Old work", 2), waiting("Watch CI", 1)] },
      ]),
    );
    const title = (n: number) => row(`acme/app#${n}`)?.querySelector(".pr-row__title") as HTMLElement;
    const tag1 = within(title(1)).getByText("T3 working");
    expect(tag1).toHaveAttribute("title", "An agent is working on this pull request in T3 Code");
    expect(title(1)).not.toHaveTextContent("T3 waiting");
    expect(title(2)).not.toHaveTextContent("T3");
    expect(title(3)).not.toHaveTextContent("T3");
    const tag4 = within(title(4)).getByText("T3 waiting");
    expect(tag4).toHaveClass("pr-row__t3", "waiting");
    expect(tag4).toHaveAttribute("title", "A T3 Code thread is watching this pull request and resumes when it changes");
    // Not confused with the bot status column.
    expect(row("acme/app#1")?.querySelector(".pr-row__status")).not.toHaveTextContent("T3");
  });

  it("marks merge conflicts with an icon right of the T3 tag", async () => {
    await setup(
      snapshotWith([
        { hasConflicts: true, t3Threads: [working("Fix CI")] },
        { hasConflicts: true, baseBranch: "" },
        { hasConflicts: false },
      ]),
    );
    const title = (n: number) => row(`acme/app#${n}`)?.querySelector(".pr-row__title") as HTMLElement;
    const mark = within(title(1)).getByRole("img", { name: "Merge conflicts with main" });
    expect(mark).toHaveClass("pr-row__conflicts");
    expect(mark).toHaveAttribute("title", "Merge conflicts with main");
    // Last on the title line, right after the T3 tag.
    expect(title(1).lastElementChild).toBe(mark);
    expect(mark.previousElementSibling).toHaveTextContent("T3 working");
    // Without a T3 tag it follows the title; with no base branch reported it still names one.
    expect(within(title(2)).getByRole("img", { name: "Merge conflicts with the base branch" })).toBeInTheDocument();
    expect(title(3).querySelector(".pr-row__conflicts")).toBeNull();
  });

  it("lists threads in the detail, working first, then waiting, then idle", async () => {
    const user = userEvent.setup();
    await setup(
      snapshotWith([
        { t3Threads: [idle("Old work", 2), waiting("Watch CI", 1), working("Fix CI")] },
        { t3Threads: [idle("Old work", 2, true)] },
        { t3Threads: [waiting("Wait on build", 3, false)] },
      ]),
    );
    const one = fact(await openDetail(user, 1), "T3 Code");
    expect(within(one as HTMLElement).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Fix CIworking now",
      "Watch CIwaiting, last active 1 h ago · watching this PR",
      "Old workidle, last active 2 h ago",
    ]);
    expect(fact(await openDetail(user, 2), "T3 Code")).toHaveTextContent(
      "Old workidle, last active 2 h ago · watching this PR",
    );
    expect(fact(await openDetail(user, 3), "T3 Code")).toHaveTextContent("Wait on buildwaiting, last active 3 h ago");
  });

  it("says No thread when T3 Code is available and omits the fact when it is not", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{ t3Threads: [] }]));
    expect(fact(await openDetail(user, 1), "T3 Code")).toHaveTextContent("T3 CodeNo thread");
  });

  it("omits the fact when T3 Code is not available", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{ t3Threads: [] }], { t3Available: false }));
    expect(fact(await openDetail(user, 1), "T3 Code")).toBeNull();
  });
});

describe("run buttons", () => {
  it("shows none when the project offers no example app", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{}]));
    const detail = await openDetail(user, 1);
    expect(within(detail).queryByRole("button", { name: /^Run on/ })).toBeNull();
  });

  it("shows one button per offered example app, beside the other actions", async () => {
    const user = userEvent.setup();
    const { store } = await setup(snapshotWith([{}]), {
      runTargets: async () => [{ repo: "acme/app", clonePath: "/src/app", cloneIsSet: true, platforms: ["ios"] }],
    });
    await act(async () => {
      await store.getState().loadRunTargets();
    });
    const detail = await openDetail(user, 1);
    expect(within(detail).getByRole("button", { name: "Run on iOS" })).toBeInTheDocument();
    expect(within(detail).queryByRole("button", { name: "Run on Android" })).toBeNull();
  });
});


describe("self-review button", () => {
  const post = /^Post \/self-reviewed$/;

  it("is the emphasised action when the gate wants it, and plain when it is only possible", async () => {
    const user = userEvent.setup();
    await setup(
      snapshotWith([
        { selfReview: { state: "wanted", postedAt: null } },
        { selfReview: { state: "possible", postedAt: null } },
      ]),
    );
    const wanted = within(await openDetail(user, 1)).getByRole("button", { name: post });
    expect(wanted).toHaveClass("btn", "primary");
    // The command is set in the monospace font.
    expect(within(wanted).getByText("/self-reviewed")).toHaveClass("mono");

    const possible = within(await openDetail(user, 2)).getByRole("button", { name: post });
    expect(possible).toHaveClass("btn");
    expect(possible).not.toHaveClass("primary");
  });

  it("is absent when there is nothing to offer", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{ selfReview: { state: "unavailable", postedAt: null } }]));
    const detail = await openDetail(user, 1);
    expect(within(detail).queryByRole("button", { name: post })).toBeNull();
    expect(within(detail).queryByRole("button", { name: "Post again" })).toBeNull();
    expect(detail).not.toHaveTextContent("Self-review posted");
  });

  it("says when it was posted and offers Post again, which opens the confirmation", async () => {
    const user = userEvent.setup();
    const postedAt = new Date(Date.now() - 3 * 60_000 - 5_000).toISOString();
    const selfReviewCheck = vi.fn<Api["selfReviewCheck"]>();
    await setup(snapshotWith([{ selfReview: { state: "posted", postedAt } }]), { selfReviewCheck });
    const detail = await openDetail(user, 1);
    expect(detail).toHaveTextContent("Self-review posted 3 min ago, waiting for the gate");
    expect(within(detail).queryByRole("button", { name: post })).toBeNull();
    const again = within(detail).getByRole("button", { name: "Post again" });
    expect(again).not.toHaveClass("primary");
    selfReviewCheck.mockReturnValue(new Promise(() => {}));
    await user.click(again);
    expect(screen.getByRole("dialog", { name: "Post /self-reviewed on #1?" })).toBeInTheDocument();
    expect(selfReviewCheck).toHaveBeenCalledWith("acme/app#1");
  });

  it("checks fresh every time the confirmation opens, and returns focus to the button on close", async () => {
    const user = userEvent.setup();
    const selfReviewCheck = vi.fn<Api["selfReviewCheck"]>(() => new Promise(() => {}));
    await setup(snapshotWith([{ selfReview: { state: "wanted", postedAt: null } }]), { selfReviewCheck });
    const button = within(await openDetail(user, 1)).getByRole("button", { name: post });
    await user.click(button);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(button).toHaveFocus();
    await user.click(button);
    expect(selfReviewCheck).toHaveBeenCalledTimes(2);
  });

  it("is not triggered by keyboard shortcuts on the row", async () => {
    const user = userEvent.setup();
    const selfReviewCheck = vi.fn<Api["selfReviewCheck"]>();
    const postSelfReview = vi.fn<Api["postSelfReview"]>();
    await setup(snapshotWith([{ selfReview: { state: "wanted", postedAt: null } }]), {
      selfReviewCheck,
      postSelfReview,
    });
    await openDetail(user, 1);
    await user.keyboard("{Enter}");
    expect(selfReviewCheck).not.toHaveBeenCalled();
    expect(postSelfReview).not.toHaveBeenCalled();
  });

  it("adds the self-review to the row's next step only when the gate wants it", async () => {
    const step = { stage: "waiting-self-review", label: "Your move", actor: "author" as const, detail: null };
    await setup(
      snapshotWith([
        { nextStep: step, selfReview: { state: "wanted", postedAt: null } },
        { nextStep: step, selfReview: { state: "possible", postedAt: null } },
      ]),
    );
    const sub = (n: number) => row(`acme/app#${n}`)?.querySelector(".pr-row__sub");
    expect(sub(1)).toHaveTextContent(/· Your move: self-review$/);
    expect(sub(2)).toHaveTextContent(/· Your move$/);
    // No posting from the row itself.
    expect(row("acme/app#1")?.querySelector("button")).toBeNull();
  });
});
