import { describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi, HEAD_MOVED_MESSAGE, SIMULATED_PUSH_SHA, type MockApiOptions } from "../../mock/mockApi";
import { mockSnapshot } from "../../mock/snapshot";
import { createAppStore, StoreProvider } from "../../state/store";
import type { Api } from "../../api";
import type { NextStep, PullRequest, SelfReviewCheck, Snapshot } from "../../types";
import { PrTable } from "./PrTable";

const HEAD = "aaaa111122223333444455556666777788889999";
const SELF_REVIEW_GATE = "Bots are done - your move: post /self-reviewed.";
const wantedStep: NextStep = { stage: "waiting-self-review", label: "Your move", actor: "author", detail: SELF_REVIEW_GATE };

/** One project, every PR mine, with the gate asking for the self-review unless patched. */
function snapshotWith(patches: Partial<PullRequest>[]): Snapshot {
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
      isDraft: false,
      bot: { state: "approved", at: null },
      nextStep: wantedStep,
      selfReview: { state: "wanted", postedAt: null },
      t3Threads: [],
      ...patch,
    };
  });
  return { ...base, projects: [{ repo: "acme/app", openCount: prs.length, prs, error: null }] };
}

type Overrides = Partial<Api> | ((mock: Api) => Partial<Api>);

async function setup(snapshot: Snapshot, overrides: Overrides = {}, options: MockApiOptions = {}) {
  const mock = createMockApi({ latencyMs: 0, snapshot, ...options });
  const api: Api = { ...mock, ...(typeof overrides === "function" ? overrides(mock) : overrides) };
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
    </StoreProvider>,
  );
  return { store, api, mock };
}

const row = (id: string) => document.querySelector<HTMLElement>(`[data-pr-row="${id}"]`);

async function openDialog(user: ReturnType<typeof userEvent.setup>, n = 1, button = "Post /self-reviewed") {
  await user.click(row(`acme/app#${n}`) as HTMLElement);
  const detail = screen.getByRole("region", { name: `Details for #${n}` });
  await user.click(within(detail).getByRole("button", { name: button }));
  return screen.getByRole("dialog");
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const check = (patch: Partial<SelfReviewCheck> = {}): SelfReviewCheck => ({
  id: "acme/app#1",
  headSha: HEAD,
  comment: `/self-reviewed ${HEAD}`,
  canPost: true,
  reason: null,
  alreadyPostedAt: null,
  gate: SELF_REVIEW_GATE,
  diffUrl: "https://github.com/acme/app/pull/1/files",
  ...patch,
});

const buttonNames = (dialog: HTMLElement) => within(dialog).getAllByRole("button").map((b) => b.textContent);

describe("Self-review confirmation", () => {
  it("checks first, then shows the commit, the gate's words and exactly what will be posted, with focus on Cancel", async () => {
    const user = userEvent.setup();
    const pending = deferred<SelfReviewCheck>();
    const selfReviewCheck = vi.fn(() => pending.promise);
    await setup(snapshotWith([{}]), { selfReviewCheck });
    const dialog = await openDialog(user);
    expect(dialog).toHaveAccessibleName("Post /self-reviewed on #1?");
    expect(dialog).toHaveAccessibleDescription("PR 1");
    expect(within(dialog).getByRole("status")).toHaveTextContent("Checking with GitHub…");
    expect(within(dialog).queryByRole("button", { name: "Post /self-reviewed" })).toBeNull();
    expect(selfReviewCheck).toHaveBeenCalledWith("acme/app#1");

    await act(async () => pending.resolve(check()));
    expect(within(dialog).queryByText("Checking with GitHub…")).toBeNull();
    expect(dialog).toHaveTextContent("Commitaaaa111");
    expect(dialog).toHaveTextContent(`PR Hygiene says${SELF_REVIEW_GATE}`);
    // The gate asks for it: no note that posting will not move it on.
    expect(dialog).not.toHaveTextContent("will not move the gate on");
    expect(within(dialog).getByText("This comment will be posted on the pull request as you:")).toBeVisible();
    const comment = dialog.querySelector("pre") as HTMLElement;
    expect(comment.textContent).toBe(`/self-reviewed ${HEAD}`);
    expect(comment).toHaveClass("mono");
    expect(comment).toHaveAccessibleName("This comment will be posted on the pull request as you:");
    expect(dialog).toHaveTextContent(
      "It is your statement that you have read the final diff of commit aaaa111 yourself.",
    );
    expect(buttonNames(dialog)).toEqual(["Open the diff on GitHub", "Cancel", "Post /self-reviewed"]);
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
  });

  it("says posting will not move the gate on when the gate is not asking for it", async () => {
    const user = userEvent.setup();
    const gate = "Waiting for bot review - thepastaclaw not yet.";
    await setup(snapshotWith([{}]), { selfReviewCheck: async () => check({ gate }) });
    const dialog = await openDialog(user);
    expect(await within(dialog).findByText(gate)).toBeVisible();
    expect(dialog).toHaveTextContent(
      "The gate is not asking for the self-review now, so posting it will not move the gate on by itself.",
    );
  });

  it("leaves the gate out when it says nothing", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{}]), { selfReviewCheck: async () => check({ gate: null }) });
    const dialog = await openDialog(user);
    await within(dialog).findByRole("button", { name: "Post /self-reviewed" });
    expect(dialog).not.toHaveTextContent("PR Hygiene says");
    expect(dialog).not.toHaveTextContent("will not move the gate on");
  });

  it("shows the reason and no post button when it cannot be posted", async () => {
    const user = userEvent.setup();
    const postSelfReview = vi.fn(async () => {});
    const reason = "The pull request is closed.";
    await setup(snapshotWith([{}]), { postSelfReview }, { selfReviewBlocked: reason });
    const dialog = await openDialog(user);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(`Cannot post: ${reason}`);
    expect(buttonNames(dialog)).toEqual(["Open the diff on GitHub", "Close"]);
    expect(dialog.querySelector("pre")).toBeNull();
    await user.keyboard("{Enter}");
    expect(postSelfReview).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says when the user already posted one for this commit, and offers Post again", async () => {
    const user = userEvent.setup();
    const postedAt = new Date(Date.now() - 2 * 3_600_000 - 60_000).toISOString();
    await setup(snapshotWith([{}]), {}, { selfReviewPostedAt: postedAt });
    const dialog = await openDialog(user);
    expect(await within(dialog).findByText("You posted a self-review for this commit 2 h ago.")).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Post again" })).toBeEnabled();
    expect(within(dialog).queryByRole("button", { name: "Post /self-reviewed" })).toBeNull();
  });

  it("opens the diff and stays open", async () => {
    const user = userEvent.setup();
    const openUrl = vi.fn(async () => {});
    await setup(snapshotWith([{}]), { openUrl });
    const dialog = await openDialog(user);
    await user.click(await within(dialog).findByRole("button", { name: "Open the diff on GitHub" }));
    expect(openUrl).toHaveBeenCalledWith("https://github.com/acme/app/pull/1/files");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("shows a failed check with Retry", async () => {
    const user = userEvent.setup();
    const selfReviewCheck = vi
      .fn<Api["selfReviewCheck"]>()
      .mockRejectedValueOnce("GitHub could not be reached.")
      .mockResolvedValueOnce(check());
    await setup(snapshotWith([{}]), { selfReviewCheck });
    const dialog = await openDialog(user);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Could not check whether the self-review can be posted: GitHub could not be reached.",
    );
    expect(within(dialog).queryByRole("button", { name: "Post /self-reviewed" })).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Retry" }));
    expect(await within(dialog).findByRole("button", { name: "Post /self-reviewed" })).toBeEnabled();
    expect(selfReviewCheck).toHaveBeenCalledTimes(2);
  });

  it("does not post on Enter while Cancel has focus", async () => {
    const user = userEvent.setup();
    const postSelfReview = vi.fn(async () => {});
    await setup(snapshotWith([{}]), { postSelfReview });
    const dialog = await openDialog(user);
    await within(dialog).findByRole("button", { name: "Post /self-reviewed" });
    await user.keyboard("{Enter}");
    expect(postSelfReview).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("on a refusal shows why, stays open and checks again, showing the new commit", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([{}]), {}, { selfReviewPushOnce: true });
    const dialog = await openDialog(user);
    await user.click(await within(dialog).findByRole("button", { name: "Post /self-reviewed" }));
    expect(await within(dialog).findByText(`The comment was not posted: ${HEAD_MOVED_MESSAGE}`)).toBeVisible();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    // The fresh check covers the commit that was just pushed.
    await waitFor(() => expect(dialog.querySelector("pre")?.textContent).toBe(`/self-reviewed ${SIMULATED_PUSH_SHA}`));
    expect(dialog).toHaveTextContent(`Commit${SIMULATED_PUSH_SHA.slice(0, 7)} (new commits were pushed)`);
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(within(dialog).getByRole("button", { name: "Post /self-reviewed" })).toBeEnabled();
  });

  it("re-checks after a refusal", async () => {
    const user = userEvent.setup();
    const selfReviewCheck = vi
      .fn<Api["selfReviewCheck"]>()
      .mockResolvedValueOnce(check())
      .mockResolvedValueOnce(check({ headSha: SIMULATED_PUSH_SHA, comment: `/self-reviewed ${SIMULATED_PUSH_SHA}` }));
    const postSelfReview = vi.fn(async () => {
      throw HEAD_MOVED_MESSAGE;
    });
    await setup(snapshotWith([{}]), { selfReviewCheck, postSelfReview });
    const dialog = await openDialog(user);
    await user.click(await within(dialog).findByRole("button", { name: "Post /self-reviewed" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(HEAD_MOVED_MESSAGE);
    await waitFor(() => expect(selfReviewCheck).toHaveBeenCalledTimes(2));
    expect(postSelfReview).toHaveBeenCalledWith("acme/app#1", HEAD);
  });

  it("posts once with the checked head, even on a double click, then closes; the detail says posted", async () => {
    const user = userEvent.setup();
    const gate = deferred<void>();
    const postSelfReview = vi.fn<Api["postSelfReview"]>();
    await setup(snapshotWith([{}]), (mock) => {
      postSelfReview.mockImplementation(async (id, sha) => {
        await gate.promise;
        return mock.postSelfReview(id, sha);
      });
      return { postSelfReview };
    });
    const dialog = await openDialog(user);
    const opener = screen.getByRole("region", { name: "Details for #1" }).querySelector(".pr-self-review button");
    const confirm = await within(dialog).findByRole("button", { name: "Post /self-reviewed" });
    await user.dblClick(confirm);
    expect(postSelfReview).toHaveBeenCalledTimes(1);
    expect(postSelfReview).toHaveBeenCalledWith("acme/app#1", HEAD);
    expect(within(dialog).getByRole("button", { name: "Posting…" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    await act(async () => gate.resolve());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const detail = screen.getByRole("region", { name: "Details for #1" });
    expect(detail).toHaveTextContent("Self-review posted just now, waiting for the gate");
    const again = within(detail).getByRole("button", { name: "Post again" });
    // The same button, so focus is back on the opener.
    expect(again).toBe(opener);
    expect(again).toHaveFocus();
    expect(postSelfReview).toHaveBeenCalledTimes(1);
  });
});
