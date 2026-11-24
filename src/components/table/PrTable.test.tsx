import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { mockSnapshot } from "../../mock/snapshot";
import { createAppStore, StoreProvider } from "../../state/store";
import type { Api } from "../../api";
import type { BotReview, PullRequest, Snapshot } from "../../types";
import { PrTable } from "./PrTable";

async function setup(snapshot?: Snapshot, patchApi?: (api: Api) => void) {
  const api = createMockApi({ latencyMs: 0, snapshot });
  patchApi?.(api);
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
    </StoreProvider>,
  );
  return { store, api };
}

const rows = () => document.querySelectorAll<HTMLElement>("[data-pr-row]");
const rowIds = () => [...rows()].map((r) => r.dataset.prRow);
const row = (id: string) => [...rows()].find((r) => r.dataset.prRow === id) as HTMLElement;
const groupTitles = () => [...document.querySelectorAll(".pr-group__title")].map((e) => e.textContent);

const NO_FINDINGS: PullRequest["findingCounts"] = {
  blocking: { total: 0, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 },
  suggestion: { total: 0, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 },
  nitpick: { total: 0, agreed: 0, notAgreed: 0, awaiting: 0, coderabbit: 0 },
};

/** The status cell's text, the time, and the live line under it (null when there is none). */
function cells(r: HTMLElement): [string | null | undefined, string | null | undefined, string | null] {
  return [
    r.querySelector(".pr-row__status")?.textContent,
    r.querySelector(".pr-row__when")?.textContent,
    r.querySelector(".pr-row__live")?.textContent ?? null,
  ];
}

/** The finding bubbles in a row's two slots, by accessible name; null for an empty slot. */
function bubbles(r: HTMLElement): [string | null, string | null] {
  const name = (slot: string) =>
    r.querySelector(`[data-slot="${slot}"] [role="img"]`)?.getAttribute("aria-label") ?? null;
  return [name("blocking"), name("suggestion")];
}

/** A snapshot with one PR per bot state, all mine, in one project. */
function everyStateSnapshot(): Snapshot {
  const now = Date.now();
  const base = mockSnapshot(now);
  const template = base.projects[0].prs[0];
  const at = (s: number) => new Date(now + s * 1000).toISOString();
  const bots: [number, BotReview][] = [
    [
      1,
      {
        state: "reviewing",
        phase: "phase1",
        phaseLabel: "Phase 1",
        progress: null,
        finishesAt: at(600),
        startedAt: at(-60),
        overdue: false,
        tier: "critical",
        priority: false,
        waitedSeconds: 30,
        steps: [
          { key: "triage", label: "Triage", status: "done", detail: null },
          { key: "phase1", label: "Phase 1", status: "running", detail: "1 of 2 lanes" },
          { key: "publish", label: "Publish", status: "pending", detail: null },
        ],
        findings: { blocking: 1, suggestion: 2, nitpick: 0 },
      },
    ],
    [2, { state: "queued", position: null, startsAt: null, finishesAt: null, reason: null, priority: false }],
    [3, { state: "approved", at: at(-7200) }],
    [4, { state: "changesRequested", at: at(-300) }],
    [5, { state: "commented", at: null }],
    [6, { state: "draft" }],
    [7, { state: "none" }],
    [8, { state: "other", title: "Review cancelled" }],
    [9, { ...(base.projects[0].prs[0].bot as Extract<BotReview, { state: "reviewing" }>), overdue: true }],
  ];
  // Reviewed PRs came in at the verdict's time; the rest were never reviewed.
  const prs: PullRequest[] = bots.map(([n, bot]) => ({
    ...template,
    id: `acme/app#${n}`,
    repo: "acme/app",
    number: n,
    title: `PR ${n}`,
    url: `https://github.com/acme/app/pull/${n}`,
    bot,
    lastReviewAt: "at" in bot ? bot.at : null,
    findingCounts: NO_FINDINGS,
  }));
  return { ...base, projects: [{ repo: "acme/app", openCount: 9, prs, error: null }] };
}

describe("PrTable", () => {
  it("shows the toolbar and the rows for Mine, grouped by project", async () => {
    await setup();
    expect(screen.getByPlaceholderText("Filter pull requests")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Project" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Finishes first" })).toHaveAttribute("aria-pressed", "true");
    expect(rows()).toHaveLength(11);
    expect(groupTitles()).toEqual(["dashpay/platform", "dashpay/dips"]);
    expect(screen.getByText("1 reviewing · 4 changes requested · 1 commented · 2 approved · 2 draft")).toBeInTheDocument();
    // Finishes first: the running review leads the platform group.
    expect(rowIds()[0]).toBe("dashpay/platform#6033");
  });

  it("shows the rows of each view", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "following" }));
    expect(rowIds()).toEqual([
      "dashpay/dash-evo-tool#1161",
      "dashpay/dash-evo-tool#1213",
      "dashpay/dash-evo-tool#1219",
      "dashpay/dash-evo-tool#1207",
      "dashpay/dash#7887",
    ]);
    expect(within(row("dashpay/dash-evo-tool#1161")).getByText(/lklimek/)).toBeInTheDocument();
    act(() => store.getState().setView({ kind: "needsMe" }));
    expect(rows()).toHaveLength(7);
    expect(row("dashpay/platform#6022").querySelector('[role="img"]')).toHaveAccessibleName("Needs you: CI failing");
    expect(row("dashpay/dash#7887").querySelector('[role="img"]')).toHaveAccessibleName(
      "Needs you: Your review is wanted",
    );
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    expect(rowIds()).toEqual(["dashpay/dips#195"]);
  });

  it("switches grouping and sorting", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.click(screen.getByRole("button", { name: "Status" }));
    expect(store.getState().groupBy).toBe("status");
    expect(groupTitles()).toEqual(["Reviewing", "Changes requested", "Comments only", "Approved", "Draft", "No bot review"]);
    // Rows show their repo when grouped by status.
    expect(row("dashpay/dips#195")).toHaveTextContent("dashpay/dips · #195");

    await user.click(screen.getByRole("button", { name: "Project" }));
    await user.click(screen.getByRole("button", { name: "Updated" }));
    expect(store.getState().sortBy).toBe("updated");
    expect(rowIds().slice(0, 3)).toEqual(["dashpay/platform#6033", "dashpay/platform#6038", "dashpay/platform#6022"]);
  });

  it("shows the status and time for every bot state when there are no findings", async () => {
    await setup(everyStateSnapshot());
    const c = (n: number) => cells(row(`acme/app#${n}`));
    expect(c(1)).toEqual(["First review", "~10 min", "Reviewing · Phase 1"]);
    expect(c(2)).toEqual(["First review", "no estimate", "Queued"]);
    expect(c(3)).toEqual(["No findings", "2 h ago", null]);
    expect(c(4)).toEqual(["No findings", "5 min ago", null]);
    expect(c(5)).toEqual(["No findings", "—", null]);
    expect(c(6)).toEqual(["Not reviewed", "—", null]);
    expect(c(7)).toEqual(["No bot review", "—", null]);
    expect(c(8)).toEqual(["Review cancelled", "—", null]);
    expect(c(9)).toEqual(["First review", "running late", "Reviewing · Phase 1"]);
    expect(row("acme/app#1").querySelector(".pr-row__time strong")).not.toBeNull();
    expect(row("acme/app#3").querySelector(".pr-row__time")).toHaveAttribute(
      "title",
      expect.stringMatching(/^Last review /),
    );
    // The verdict pill is gone.
    expect(screen.queryByText("Approved")).toBeNull();
    expect(screen.queryByText("Changes requested")).toBeNull();
  });

  it("shows blockers and suggestions as bubbles in fixed slots", async () => {
    await setup();
    const b = (id: string) => bubbles(row(id));
    // All settled with agreement: the quiet form.
    expect(b("dashpay/platform#6038")).toEqual([
      "2 blockers in the last review round: 2 agreed, 0 not agreed, 0 open",
      "4 suggestions in the last review round: 4 agreed, 0 not agreed, 0 open",
    ]);
    expect(row("dashpay/platform#6038").querySelector(".tally.blocking")).toHaveClass("quiet");
    // Mixed agreed, not agreed and open: the strong form.
    expect(b("dashpay/platform#5968")).toEqual([
      "4 blockers in the last review round: 1 agreed, 1 not agreed, 2 open",
      "6 suggestions in the last review round: 2 agreed, 3 not agreed, 1 open",
    ]);
    expect(row("dashpay/platform#5968").querySelector(".tally.blocking")).toHaveClass("open");
    // Nothing settled.
    expect(b("dashpay/platform#6034")).toEqual([
      "3 blockers in the last review round: 0 agreed, 0 not agreed, 3 open",
      "2 suggestions in the last review round: 0 agreed, 0 not agreed, 2 open",
    ]);
    // Blockers only, suggestions only: the other slot stays, empty.
    expect(b("dashpay/platform#6023")).toEqual(["1 blocker in the last review round: 0 agreed, 0 not agreed, 1 open", null]);
    expect(row("dashpay/platform#6023").querySelector('[data-slot="suggestion"]')).toBeEmptyDOMElement();
    expect(b("dashpay/platform#6022")).toEqual([null, "3 suggestions in the last review round: 1 agreed, 0 not agreed, 2 open"]);
    // Only not agreed, all settled: quiet, with the thumbs-down still there.
    expect(b("dashpay/platform#5750")[0]).toBe("1 blocker in the last review round: 0 agreed, 1 not agreed, 0 open");
    expect(row("dashpay/platform#5750").querySelector(".tally.blocking.quiet .tally__part.down")).toHaveTextContent("1");
    // No CodeRabbit findings: no rabbit.
    expect(row("dashpay/platform#5968").querySelector(".tally__rabbit")).toBeNull();
    // Large numbers, CodeRabbit's included: a rabbit in each bubble.
    const bigLabel =
      "118 suggestions in the last review round: 104 agreed, 9 not agreed, 5 open. 76 from CodeRabbit.";
    expect(b("dashpay/platform#5749")[1]).toBe(bigLabel);
    const big = row("dashpay/platform#5749").querySelector(".tally.suggestion") as HTMLElement;
    expect([...big.querySelectorAll(".tally__part")].map((p) => p.textContent)).toEqual(["9", "104"]);
    expect(big.querySelector(".tally__total")).toHaveTextContent("/118");
    expect(big.querySelector(".tally__rabbit")).not.toBeNull();
    expect(big).toHaveAttribute("title", bigLabel);
    // Reviewed with no blockers or suggestions.
    expect(cells(row("dashpay/platform#6002"))).toEqual(["No findings", "12 h ago", null]);
    // A draft the bot skipped says so beside its tag; no bot review keeps its text.
    expect(cells(row("dashpay/platform#5967"))[0]).toBe("Not reviewed");
    expect(cells(row("dashpay/dips#195"))[0]).toBe("No bot review");
  });

  it("shows a rabbit in each bubble whose total includes CodeRabbit's findings", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "following" }));
    const rabbits = (id: string) =>
      (["blocking", "suggestion"] as const).map((sev) => {
        const tally = row(id).querySelector(`.tally.${sev}`);
        return tally ? tally.querySelector(".tally__rabbit") != null : null;
      });
    // CodeRabbit in the suggestions only, and that bubble is all CodeRabbit's.
    expect(rabbits("dashpay/dash-evo-tool#1213")).toEqual([false, true]);
    expect(bubbles(row("dashpay/dash-evo-tool#1213"))).toEqual([
      "1 blocker in the last review round: 1 agreed, 0 not agreed, 0 open",
      "2 suggestions in the last review round: 1 agreed, 0 not agreed, 1 open. 2 from CodeRabbit.",
    ]);
    // In the blockers only.
    expect(rabbits("dashpay/dash-evo-tool#1219")).toEqual([true, false]);
    expect(bubbles(row("dashpay/dash-evo-tool#1219"))[0]).toBe(
      "2 blockers in the last review round: 0 agreed, 1 not agreed, 1 open. 1 from CodeRabbit.",
    );
    // In both, the settled blocker in the quiet form.
    expect(rabbits("dashpay/dash-evo-tool#1207")).toEqual([true, true]);
    expect(row("dashpay/dash-evo-tool#1207").querySelector(".tally.blocking.quiet .tally__rabbit")).not.toBeNull();
    // None: no rabbit.
    expect(rabbits("dashpay/dash#7887")).toEqual([false, false]);
    // The rabbit is drawn, not read out: the bubble's name already says it.
    const rabbit = row("dashpay/dash-evo-tool#1219").querySelector(".tally__rabbit");
    expect(rabbit).toHaveAttribute("aria-hidden", "true");
  });

  it("shows earlier findings beside a live review's countdown", async () => {
    const { store } = await setup();
    const reviewing = row("dashpay/platform#6033");
    // Every thread resolved after a push the bot is reviewing now: all awaiting, quiet.
    expect(bubbles(reviewing)).toEqual([
      "5 blockers in the last review round: 0 agreed, 0 not agreed, 5 awaiting re-review, 0 open",
      "5 suggestions in the last review round: 0 agreed, 0 not agreed, 5 awaiting re-review, 0 open",
    ]);
    const waiting = reviewing.querySelector(".tally.blocking") as HTMLElement;
    expect(waiting).toHaveClass("quiet");
    expect([...waiting.querySelectorAll(".tally__part")].map((p) => p.className)).toEqual(["tally__part wait"]);
    expect(waiting.querySelector(".tally__part.wait")).toHaveTextContent("5");
    expect(waiting.querySelector(".tally__hourglass")).toHaveAttribute("aria-hidden", "true");
    expect(cells(reviewing).slice(1)).toEqual(["~20 min", "Reviewing · Phase 1"]);
    act(() => store.getState().setView({ kind: "following" }));
    expect(cells(row("dashpay/dash#7887")).slice(1)).toEqual(["~58 min", "Queued · #6"]);
    expect(bubbles(row("dashpay/dash#7887"))).toEqual([
      "2 blockers in the last review round: 1 agreed, 0 not agreed, 1 open",
      "6 suggestions in the last review round: 2 agreed, 1 not agreed, 3 awaiting re-review, 0 open",
    ]);
    // Thumbs-down, hourglass, thumbs-up, then the total.
    const mixed = row("dashpay/dash#7887").querySelector(".tally.suggestion") as HTMLElement;
    expect(mixed).toHaveClass("quiet");
    expect([...mixed.querySelectorAll(".tally__part")].map((p) => [p.className, p.textContent])).toEqual([
      ["tally__part down", "1"],
      ["tally__part wait", "3"],
      ["tally__part up", "2"],
    ]);
    expect(mixed.querySelector(".tally__total")).toHaveTextContent("/6");
    // Queued for its first review.
    expect(cells(row("dashpay/dash-evo-tool#1161"))).toEqual(["First review", "~47 min", "Queued · #3"]);
  });

  it("puts the next step on the sub-line unless the row already says it", async () => {
    await setup();
    const sub = (id: string) => row(id).querySelector(".pr-row__sub");
    // The gate wants the self-review here, so the row says what the move is.
    expect(sub("dashpay/platform#6038")).toHaveTextContent(/· Your move: self-review$/);
    // An agent works on this one in T3 Code: its "T3 working" tag says who has it, not "Your move".
    expect(row("dashpay/platform#6034").querySelector(".pr-row__t3.working")).toHaveTextContent("T3 working");
    expect(sub("dashpay/platform#6034")).not.toHaveTextContent("Your move");
    expect(sub("dashpay/platform#5750")).toHaveTextContent(/· Ready to merge$/);
    expect(sub("dashpay/platform#5749")).toHaveTextContent(/· Needs rebase$/);
    // The Draft tag already says draft, reviewed or not; the live line already says the bots are busy.
    expect(sub("dashpay/platform#5967")).not.toHaveTextContent("Draft");
    expect(sub("dashpay/platform#6002")).not.toHaveTextContent("Draft");
    expect(sub("dashpay/platform#6033")).not.toHaveTextContent("Waiting for bots");
    // No gate on this repo.
    expect(sub("dashpay/dips#195")?.textContent).toBe("#195 · +524 −1 · needs 1 human review");
  });

  it("shows the next step and its full sentence in the detail, for every bot state", async () => {
    const user = userEvent.setup();
    const snapshot = everyStateSnapshot();
    const sentence =
      "Your move: thepastaclaw requested changes on this head; dismiss the review or push a fix, then post /self-reviewed.";
    snapshot.projects[0].prs = snapshot.projects[0].prs.map((p) => ({
      ...p,
      nextStep: { stage: "waiting-self-review", label: "Your move", actor: "author", detail: sentence },
    }));
    await setup(snapshot);
    for (const n of [1, 2, 3, 4, 5, 6, 7, 8]) {
      await user.click(row(`acme/app#${n}`));
      const detail = screen.getByRole("region", { name: `Details for #${n}` });
      const fact = within(detail).getByText("Next step").closest(".pr-fact");
      expect(fact).toHaveTextContent(`Next stepYour move${sentence}`);
      expect(within(detail).getByText(sentence)).toHaveClass("selectable");
    }
  });

  it("leaves the next step out of the detail when there is none", async () => {
    const user = userEvent.setup();
    await setup();
    await user.click(row("dashpay/dips#195"));
    const detail = screen.getByRole("region", { name: "Details for #195" });
    expect(within(detail).queryByText("Next step")).toBeNull();
  });

  it("shows CI with a text label", async () => {
    await setup();
    expect(within(row("dashpay/platform#5750")).getByText("✓ CI")).toBeInTheDocument();
    expect(within(row("dashpay/platform#6022")).getByText("CI failing")).toBeInTheDocument();
  });

  it("expands a reviewing row into its progress detail", async () => {
    const user = userEvent.setup();
    const { store } = await setup(everyStateSnapshot());
    await user.click(row("acme/app#1"));
    expect(store.getState().expandedId).toBe("acme/app#1");
    expect(row("acme/app#1")).toHaveAttribute("aria-expanded", "true");
    const detail = screen.getByRole("region", { name: "Details for #1" });
    expect(within(detail).getByRole("progressbar")).toHaveClass("unknown");
    const steps = within(detail).getByRole("list", { name: "Review steps" });
    expect(steps).toHaveTextContent("✓ Triage, done");
    expect(steps).toHaveTextContent("● Phase 1 · 1 of 2 lanes, running");
    expect(steps).toHaveTextContent("Publish, not started");
    expect(detail).toHaveTextContent("Tiercritical");
    expect(detail).toHaveTextContent("Waited30 s");
    expect(detail).toHaveTextContent(/Running1 min/);
    expect(detail).toHaveTextContent("Findings so far1 blocking · 2 suggestions");
    expect(detail).toHaveTextContent("PastaClaw reviewing · No human reviewers requested");
    // Never reviewed before: no Findings or Last review facts.
    expect(within(detail).queryByText("Findings", { selector: "dt" })).toBeNull();
    expect(within(detail).queryByText("Last review", { selector: "dt" })).toBeNull();
    // Mine: no follow button.
    expect(within(detail).queryByRole("button", { name: "Follow" })).toBeNull();
    await user.click(row("acme/app#1"));
    expect(screen.queryByRole("region", { name: "Details for #1" })).toBeNull();
  });

  it("expands a queued row into its queue detail with follow actions", async () => {
    const user = userEvent.setup();
    const openUrl = vi.fn(async () => {});
    const { store } = await setup(undefined, (api) => {
      api.openUrl = openUrl;
    });
    act(() => store.getState().setView({ kind: "following" }));
    await user.click(row("dashpay/dash#7887"));
    const detail = screen.getByRole("region", { name: "Details for #7887" });
    expect(detail).toHaveTextContent("Position#6 in queue");
    expect(detail).toHaveTextContent(/Can startin ~26 min/);
    expect(detail).toHaveTextContent("Waiting fordebounce (more pushes may follow)");
    expect(detail).toHaveTextContent(/Estimated finish~58 min/);
    expect(detail).toHaveTextContent("Estimated from the bot's median review time");
    expect(detail).toHaveTextContent("UdjinM6 requested changes · knst commented");
    expect(detail).toHaveTextContent(
      "FindingsBlockers 1 of 2 settled · Suggestions 3 of 6 settled (1 not agreed), 3 awaiting re-review · Nitpicks 2 of 3 settled (last review round of each reviewer)",
    );
    expect(detail).toHaveTextContent(/Last review.+ · 5 h ago/);
    await user.click(within(detail).getByRole("button", { name: "Open on GitHub" }));
    expect(openUrl).toHaveBeenCalledWith("https://github.com/dashpay/dash/pull/7887");
    await user.click(within(detail).getByRole("button", { name: "Unfollow" }));
    expect(store.getState().config?.followed).not.toContain("dashpay/dash#7887");
  });

  it("filters, shows an empty state for no match, and clears", async () => {
    const user = userEvent.setup();
    await setup();
    const input = screen.getByPlaceholderText("Filter pull requests");
    await user.type(input, "shielded account");
    expect(rowIds()).toEqual(["dashpay/platform#5967"]);
    await user.clear(input);
    await user.type(input, "zzz");
    expect(rows()).toHaveLength(0);
    expect(screen.getByText("No pull requests match “zzz”")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear filter" }));
    expect(input).toHaveValue("");
    expect(rows()).toHaveLength(11);
  });

  it("shows an empty view with guidance", async () => {
    const snap = mockSnapshot();
    const empty = { ...snap, projects: snap.projects.map((p) => ({ ...p, prs: p.prs.filter((x) => x.isMine) })) };
    const noNeeds = {
      ...empty,
      projects: empty.projects.map((p) => ({ ...p, prs: p.prs.map((x) => ({ ...x, needsMe: false })) })),
    };
    const { store } = await setup(noNeeds);
    act(() => store.getState().setView({ kind: "following" }));
    expect(screen.getByText("No pull requests here yet")).toBeInTheDocument();
    expect(screen.getByText(/To follow someone else's pull request/)).toBeInTheDocument();
    act(() => store.getState().setView({ kind: "needsMe" }));
    expect(screen.getByText("Nothing is waiting on you.")).toBeInTheDocument();
  });

  it("moves through rows with the keyboard and toggles the detail", async () => {
    const user = userEvent.setup();
    const openUrl = vi.fn(async () => {});
    const { store } = await setup(undefined, (api) => {
      api.openUrl = openUrl;
    });
    const ids = rowIds() as string[];
    await user.keyboard("{ArrowDown}");
    expect(store.getState().focusedId).toBe(ids[0]);
    expect(row(ids[0])).toHaveFocus();
    await user.keyboard("jj");
    expect(store.getState().focusedId).toBe(ids[2]);
    await user.keyboard("k");
    expect(store.getState().focusedId).toBe(ids[1]);
    expect(row(ids[1])).toHaveClass("focused");

    await user.keyboard("{Enter}");
    expect(store.getState().expandedId).toBe(ids[1]);
    await user.keyboard("{Escape}");
    expect(store.getState().expandedId).toBeNull();
    await user.keyboard(" ");
    expect(store.getState().expandedId).toBe(ids[1]);

    await user.keyboard("o");
    expect(openUrl).toHaveBeenLastCalledWith(`https://github.com/${ids[1].replace("#", "/pull/")}`);
    openUrl.mockClear();
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(openUrl).toHaveBeenCalledTimes(1);

    // Crosses group boundaries: the last platform row, then the dips row.
    for (let i = 0; i < 20; i++) await user.keyboard("{ArrowDown}");
    expect(store.getState().focusedId).toBe("dashpay/dips#195");
  });

  it("focuses the filter with / and returns to the list with Esc", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    const input = screen.getByPlaceholderText("Filter pull requests");
    await user.keyboard("/");
    expect(input).toHaveFocus();
    await user.keyboard("dips");
    // Keys typed in the filter do not move the list.
    expect(store.getState().focusedId).toBeNull();
    expect(rows()).toHaveLength(1);
    await user.keyboard("{Escape}");
    expect(input).toHaveValue("");
    expect(rows()).toHaveLength(11);
    // Focus lands on the row the filter had narrowed to.
    expect(store.getState().focusedId).toBe("dashpay/dips#195");
    expect(row("dashpay/dips#195")).toHaveFocus();
  });

  it("ignores list keys while a dialog is open", async () => {
    const { store } = await setup();
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    document.body.appendChild(dialog);
    fireEvent.keyDown(window, { key: "j" });
    expect(store.getState().focusedId).toBeNull();
    dialog.remove();
    fireEvent.keyDown(window, { key: "j" });
    expect(store.getState().focusedId).not.toBeNull();
  });
});

