import { describe, expect, it } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { createAppStore, NO_AREA, StoreProvider, type AppStore } from "../../state/store";
import { CommandPalette } from "../palette/CommandPalette";
import { PrTable } from "./PrTable";

const PLATFORM = "dashpay/platform";

async function setup() {
  const api = createMockApi({ latencyMs: 0 });
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
      <CommandPalette />
    </StoreProvider>,
  );
  return { store, api };
}

const rows = () => [...document.querySelectorAll<HTMLElement>("[data-pr-row]")];
const rowIds = () => rows().map((r) => r.dataset.prRow);
const row = (id: string) => rows().find((r) => r.dataset.prRow === id) as HTMLElement;
const draftsSwitch = () => screen.queryByRole("button", { name: /^Drafts, / });
const milestones = () => screen.queryByRole("group", { name: "Milestones" });
const note = () => document.querySelector(".pr-hidden-note")?.textContent ?? null;
const groupMeta = (repo: string) =>
  [...document.querySelectorAll(".pr-group__head")]
    .find((h) => h.querySelector(".pr-group__title")?.textContent === repo)
    ?.querySelector(".pr-group__meta")?.textContent;
const allHidden = () => document.querySelector<HTMLElement>(".pr-empty") as HTMLElement;
const hideDrafts = (store: AppStore) => act(() => store.getState().setShowDrafts(false));

// Mine has three drafts: #5967 (never reviewed, no milestone), #5968
// (v5.2.0, reviewed before it became a draft) and #6002 (v6.0.0, reviewed,
// nothing found).
const MINE_DRAFTS = ["dashpay/platform#5967", "dashpay/platform#5968", "dashpay/platform#6002"];

describe("Draft rows", () => {
  it("start with a Draft tag that is part of the row's name", async () => {
    await setup();
    for (const id of MINE_DRAFTS) {
      const r = row(id);
      expect(r).toHaveClass("draft");
      // A stacked draft (the mock's #5967, stacked on #5750) has its ↳ arrow
      // first; the Draft tag comes straight after it.
      const stacked = r.classList.contains("stacked");
      const tag = r.querySelector(".pr-row__title > .pr-row__draft");
      expect(r.querySelector(".pr-row__title > :first-child")).toBe(
        stacked ? r.querySelector(".pr-row__stacked") : tag,
      );
      expect(tag).toHaveTextContent(/^Draft$/);
      // The test library trims each child's text, so the space after the comma is lost here.
      expect(r).toHaveAccessibleName(stacked ? /^Stacked on #\d+,\s*Draft,\s*\S/ : /^Draft,\s*\S/);
    }
  });

  it("has no tag on a pull request that is not a draft", async () => {
    await setup();
    const live = row("dashpay/platform#6033");
    expect(live).not.toHaveClass("draft");
    expect(live.querySelector(".pr-row__draft")).toBeNull();
    expect(live).not.toHaveAccessibleName(/draft/i);
  });

  it("shows a reviewed draft's tag and its finding bubbles", async () => {
    await setup();
    const r = row("dashpay/platform#5968");
    expect(r.querySelector(".pr-row__draft")).toHaveTextContent("Draft");
    expect(r.querySelector('[data-slot="blocking"] .tally.blocking')).not.toBeNull();
    expect(r.querySelector('[data-slot="suggestion"] .tally.suggestion')).not.toBeNull();
  });

  it("says Not reviewed beside the tag of a draft the bot skipped, and Draft only once", async () => {
    await setup();
    const r = row("dashpay/platform#5967");
    expect(r.querySelector(".pr-row__status")).toHaveTextContent(/^Not reviewed$/);
    expect(r.textContent?.match(/Draft/g)).toHaveLength(1);
  });

  it("says draft in the detail", async () => {
    const user = userEvent.setup();
    await setup();
    // The gate's next step says it for #5967.
    await user.click(row("dashpay/platform#5967"));
    let detail = document.getElementById("pr-detail-dashpay-platform-5967") as HTMLElement;
    expect(within(detail).getByText("Next step").nextElementSibling).toHaveTextContent(/^Draft/);
    // #5968's next step is about its review threads, so the detail adds it.
    await user.click(row("dashpay/platform#5968"));
    detail = document.getElementById("pr-detail-dashpay-platform-5968") as HTMLElement;
    expect(within(detail).getByText("Draft").nextElementSibling).toHaveTextContent("Not yet marked ready for review");
  });
});

describe("Drafts switch", () => {
  it("shows the number of drafts in the view, pressed while drafts are shown", async () => {
    await setup();
    const sw = draftsSwitch() as HTMLElement;
    expect(sw).toHaveAccessibleName("Drafts, 3 pull requests");
    expect(sw).toHaveAttribute("aria-pressed", "true");
    expect(sw.querySelector(".ms-chip__box")).toHaveTextContent("✓");
    // Not a milestone.
    expect(within(milestones() as HTMLElement).queryByRole("button", { name: /^Drafts/ })).toBeNull();
  });

  it("shows only when the view has a draft or drafts are hidden", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    expect(draftsSwitch()).toBeNull();
    // With drafts hidden it stays, so they can be shown again.
    hideDrafts(store);
    expect(draftsSwitch()).toHaveAccessibleName("Drafts, 0 pull requests");
    expect(draftsSwitch()).toHaveAttribute("aria-pressed", "false");
  });

  it("shows in a view where the milestone line does not", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dash-evo-tool" }));
    expect(milestones()).toBeNull();
    expect(draftsSwitch()).toHaveAccessibleName("Drafts, 1 pull request");
  });

  it("hides and shows every draft, and the group summary counts only what is shown", async () => {
    const user = userEvent.setup();
    await setup();
    expect(groupMeta(PLATFORM)).toBe("1 reviewing · 4 changes requested · 1 commented · 2 approved · 2 draft");
    await user.click(draftsSwitch() as HTMLElement);
    expect(draftsSwitch()).toHaveAttribute("aria-pressed", "false");
    expect(draftsSwitch()?.querySelector(".ms-chip__box")).toHaveTextContent("");
    expect(rowIds()).toHaveLength(8);
    for (const id of MINE_DRAFTS) expect(rowIds()).not.toContain(id);
    expect(groupMeta(PLATFORM)).toBe("1 reviewing · 3 changes requested · 1 commented · 2 approved");
    expect(note()).toBe("3 drafts hidden");
    // Milestone chips keep counting the whole view.
    expect(within(milestones() as HTMLElement).getByRole("button", { name: /^v6\.0\.0, / })).toHaveAccessibleName(
      "v6.0.0, 1 pull request",
    );
    // The count stays the view's drafts while they are hidden.
    expect(draftsSwitch()).toHaveAccessibleName("Drafts, 3 pull requests");

    await user.click(draftsSwitch() as HTMLElement);
    expect(rowIds()).toHaveLength(11);
    expect(note()).toBeNull();
  });

  it("is remembered in a new store on the same storage", async () => {
    const user = userEvent.setup();
    const { api } = await setup();
    await user.click(draftsSwitch() as HTMLElement);
    expect(createAppStore(api).getState().showDrafts).toBe(false);
  });
});

describe("Drafts in the list", () => {
  it("names each cause once, drafts first, then milestone, then area", async () => {
    const { store } = await setup();
    hideDrafts(store);
    // v5.2.0: #6022, #6023 and the draft #5968, counted as a draft.
    act(() => store.getState().toggleMilestone("v5.2.0"));
    // sdk: #6034, the draft #5967 (a draft), #6022 (milestone; drive-abci is on anyway).
    act(() => store.getState().toggleArea(PLATFORM, "sdk"));
    expect(note()).toBe("3 drafts hidden · 2 hidden by milestone · 1 hidden by area");
    expect(rowIds()).toHaveLength(5);
  });

  it("says when drafts alone hide everything, and shows them from there", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    hideDrafts(store);
    act(() => store.getState().setQuery("#596"));
    expect(rowIds()).toEqual([]);
    const empty = allHidden();
    expect(empty).toHaveTextContent("All 2 matching pull requests are drafts, and drafts are hidden");
    expect(within(empty).queryByRole("button", { name: /milestones|areas|Show all/ })).toBeNull();
    await user.click(within(empty).getByRole("button", { name: "Show drafts" }));
    expect(store.getState().showDrafts).toBe(true);
    expect([...rowIds()].sort()).toEqual(["dashpay/platform#5967", "dashpay/platform#5968"]);
  });

  it("names every cause when drafts, milestones and areas together hide everything", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: PLATFORM }));
    hideDrafts(store);
    for (const m of ["v5.1.0", "v5.2.0", "v6.0.0"]) act(() => store.getState().toggleMilestone(m));
    act(() => store.getState().toggleArea(PLATFORM, NO_AREA));
    expect(rowIds()).toEqual([]);
    const empty = allHidden();
    expect(empty).toHaveTextContent(
      `All 10 pull requests here are drafts or in milestones or areas of ${PLATFORM} you have switched off`,
    );
    expect(empty).toHaveTextContent("3 drafts hidden · 6 hidden by milestone · 1 hidden by area");
    const actions = within(empty)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(actions).toEqual(["Show drafts", "Show all milestones", "Show all areas"]);
    await user.click(within(empty).getByRole("button", { name: "Show drafts" }));
    // The never-reviewed draft has no milestone and its area is on.
    expect(rowIds()).toEqual(["dashpay/platform#5967"]);
  });

  it("names drafts beside a milestone that hides the same pull request", async () => {
    const { store } = await setup();
    hideDrafts(store);
    act(() => store.getState().toggleMilestone("v5.2.0"));
    act(() => store.getState().setQuery("#596"));
    const empty = allHidden();
    // #5968 is both a draft and in v5.2.0: showing drafts alone would not bring it back.
    expect(empty).toHaveTextContent("All 2 matching pull requests are drafts or in milestones you have switched off");
    expect(within(empty).getByRole("button", { name: "Show all milestones" })).toBeInTheDocument();
  });

  it("moves through visible rows only with the keyboard", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    hideDrafts(store);
    const seen = new Set<string | null>();
    for (let i = 0; i < 12; i++) {
      await user.keyboard("j");
      seen.add(store.getState().focusedId);
    }
    expect(seen.size).toBe(8);
    for (const id of MINE_DRAFTS) expect(seen).not.toContain(id);
  });

  it("switches drafts back on when the palette jumps to a hidden draft", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    hideDrafts(store);
    act(() => store.getState().toggleMilestone("v6.0.0"));
    expect(rowIds()).not.toContain("dashpay/platform#6002");
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("local node");
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("run an added local node");
    await user.keyboard("{Enter}");
    expect(store.getState().showDrafts).toBe(true);
    expect(store.getState().hiddenMilestones).toEqual([]);
    expect(rowIds()).toContain("dashpay/platform#6002");
    expect(store.getState().expandedId).toBe("dashpay/platform#6002");
  });

  it("leaves drafts hidden when the palette jumps to a pull request that is not one", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    hideDrafts(store);
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("6033");
    await user.keyboard("{Enter}");
    expect(store.getState().showDrafts).toBe(false);
  });
});
