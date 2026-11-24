import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { createAppStore, StoreProvider, type AppStore } from "../../state/store";
import { CommandPalette } from "../palette/CommandPalette";
import { PrTable } from "./PrTable";

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
const recentSwitch = () => screen.queryByRole("button", { name: /^Created recently, / });
const draftsSwitch = () => screen.queryByRole("button", { name: /^Drafts, / });
const note = () => document.querySelector(".pr-hidden-note")?.textContent ?? null;
const allHidden = () => document.querySelector<HTMLElement>(".pr-empty") as HTMLElement;
const recentOnly = (store: AppStore) => act(() => store.getState().setRecentOnly(true));
const hideDrafts = (store: AppStore) => act(() => store.getState().setShowDrafts(false));

// Mine has eleven pull requests, four of them opened in the last 3 days:
// #6033, #6038, #6022 and the draft #6002.
const MINE_RECENT = [
  "dashpay/platform#6002",
  "dashpay/platform#6022",
  "dashpay/platform#6033",
  "dashpay/platform#6038",
];

afterEach(() => {
  vi.useRealTimers();
});

describe("Created recently switch", () => {
  it("sits right after the drafts switch, off, with the number of recent pull requests in the view", async () => {
    await setup();
    const sw = recentSwitch() as HTMLElement;
    expect(sw).toHaveAccessibleName("Created recently, 4 pull requests");
    expect(sw).toHaveAttribute("aria-pressed", "false");
    expect(sw).toHaveAttribute("title", "Show only pull requests created in the last 3 days");
    expect(sw).toHaveTextContent("Created recently4");
    expect(sw.querySelector(".ms-chip__box")).toHaveTextContent("");
    expect(draftsSwitch()?.nextElementSibling).toBe(sw);
    // Off, it hides nothing.
    expect(rowIds()).toHaveLength(11);
    expect(note()).toBeNull();
  });

  it("lists only recent pull requests while ticked, and everything again when unticked", async () => {
    const user = userEvent.setup();
    await setup();
    await user.click(recentSwitch() as HTMLElement);
    const sw = recentSwitch() as HTMLElement;
    expect(sw).toHaveAttribute("aria-pressed", "true");
    expect(sw).toHaveAttribute("title", "Show pull requests of any age");
    expect(sw.querySelector(".ms-chip__box")).toHaveTextContent("✓");
    expect([...rowIds()].sort()).toEqual(MINE_RECENT);
    expect(note()).toBe("7 older than 3 days hidden");
    // The count stays the view's recent pull requests.
    expect(sw).toHaveAccessibleName("Created recently, 4 pull requests");

    await user.click(sw);
    expect(rowIds()).toHaveLength(11);
    expect(note()).toBeNull();
  });

  it("shows only when the view has a recent pull request or the switch is on", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    expect(recentSwitch()).toBeNull();
    // On, it stays, so it can be turned off again.
    recentOnly(store);
    expect(recentSwitch()).toHaveAccessibleName("Created recently, 0 pull requests");
    expect(recentSwitch()).toHaveAttribute("aria-pressed", "true");
  });

  it("counts a recent pull request in Following", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "following" }));
    expect(recentSwitch()).toHaveAccessibleName("Created recently, 1 pull request");
  });

  it("is remembered in a new store on the same storage", async () => {
    const user = userEvent.setup();
    const { api } = await setup();
    await user.click(recentSwitch() as HTMLElement);
    expect(createAppStore(api).getState().recentOnly).toBe(true);
  });

  it("re-evaluates the list as time passes", async () => {
    vi.useFakeTimers({ now: Date.parse("2026-11-22T10:00:00Z") });
    const { store } = await setup();
    recentOnly(store);
    expect(rowIds()).toHaveLength(4);
    // A day later only #6038 (opened 20 hours before) is still recent.
    act(() => vi.setSystemTime(Date.parse("2026-11-23T11:00:00Z")));
    act(() => vi.advanceTimersByTime(30_000));
    expect(rowIds()).toEqual(["dashpay/platform#6038"]);
    expect(recentSwitch()).toHaveAccessibleName("Created recently, 1 pull request");
    expect(note()).toBe("10 older than 3 days hidden");
  });
});

describe("Created recently in the list", () => {
  it("names each cause once: drafts, then age, then milestone", async () => {
    const { store } = await setup();
    hideDrafts(store);
    recentOnly(store);
    // Drafts: #5967, #5968, #6002 (recent, but a draft first).
    expect(note()).toBe("3 drafts hidden · 5 older than 3 days hidden");
    // v5.2.0 holds #6022 (recent), #6023 (older) and the draft #5968.
    act(() => store.getState().toggleMilestone("v5.2.0"));
    expect(note()).toBe("3 drafts hidden · 5 older than 3 days hidden · 1 hidden by milestone");
    expect([...rowIds()].sort()).toEqual(["dashpay/platform#6033", "dashpay/platform#6038"]);
  });

  it("says when age alone hides everything, and shows them from there", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    recentOnly(store);
    act(() => store.getState().setQuery("#596"));
    expect(rowIds()).toEqual([]);
    const empty = allHidden();
    expect(empty).toHaveTextContent(
      "All 2 matching pull requests are older than 3 days, and only recent pull requests are shown",
    );
    expect(empty.querySelector(".pr-empty__text")).toBeNull();
    const actions = within(empty)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(actions).toEqual(["Show all"]);
    await user.click(within(empty).getByRole("button", { name: "Show all" }));
    expect(store.getState().recentOnly).toBe(false);
    expect([...rowIds()].sort()).toEqual(["dashpay/platform#5967", "dashpay/platform#5968"]);
  });

  it("says it for the one pull request of a project", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    recentOnly(store);
    expect(allHidden()).toHaveTextContent(
      "The one pull request here is older than 3 days, and only recent pull requests are shown",
    );
  });

  it("names drafts and age together, and each has its action", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    hideDrafts(store);
    recentOnly(store);
    act(() => store.getState().setQuery("#596"));
    const empty = allHidden();
    expect(empty).toHaveTextContent("All 2 matching pull requests are drafts or older than 3 days");
    expect(empty).toHaveTextContent("2 drafts hidden");
    const actions = within(empty)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(actions).toEqual(["Show drafts", "Show older pull requests"]);
    await user.click(within(empty).getByRole("button", { name: "Show older pull requests" }));
    expect(store.getState().recentOnly).toBe(false);
    // Still all drafts, so the list says that now.
    expect(allHidden()).toHaveTextContent("All 2 matching pull requests are drafts, and drafts are hidden");
  });

  it("names drafts, age and milestones in one sentence", async () => {
    const { store } = await setup();
    hideDrafts(store);
    recentOnly(store);
    act(() => store.getState().toggleMilestone("v5.2.0"));
    // #6022 is recent and in v5.2.0; #6023 is older and in v5.2.0.
    act(() => store.getState().setQuery("#602"));
    const empty = allHidden();
    expect(empty).toHaveTextContent(
      "All 2 matching pull requests are older than 3 days or in milestones you have switched off",
    );
    expect(empty).toHaveTextContent("1 older than 3 days hidden · 1 hidden by milestone");
    act(() => store.getState().setQuery("#600"));
    expect(allHidden()).toHaveTextContent("The one matching pull request is a draft, and drafts are hidden");
    act(() => store.getState().setQuery("#59"));
    // #5967, #5968 (drafts, older, #5968 in v5.2.0).
    expect(allHidden()).toHaveTextContent(
      "All 2 matching pull requests are drafts, older than 3 days or in milestones you have switched off",
    );
    expect(
      within(allHidden())
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Show drafts", "Show older pull requests", "Show all milestones"]);
  });

  it("says more than one filter for one pull request hidden by drafts and age", async () => {
    const { store } = await setup();
    hideDrafts(store);
    recentOnly(store);
    act(() => store.getState().setQuery("#5967"));
    expect(allHidden()).toHaveTextContent("The one matching pull request is hidden by more than one filter");
    expect(allHidden()).toHaveTextContent("1 draft hidden");
  });

  it("turns the switch off when the palette jumps to an older pull request", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    recentOnly(store);
    expect(rowIds()).not.toContain("dashpay/platform#5750");
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("5750");
    await user.keyboard("{Enter}");
    expect(store.getState().recentOnly).toBe(false);
    expect(rowIds()).toContain("dashpay/platform#5750");
    expect(store.getState().expandedId).toBe("dashpay/platform#5750");
  });

  it("leaves the switch on when the palette jumps to a recent pull request", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    recentOnly(store);
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("6033");
    await user.keyboard("{Enter}");
    expect(store.getState().recentOnly).toBe(true);
    expect(store.getState().expandedId).toBe("dashpay/platform#6033");
  });
});
