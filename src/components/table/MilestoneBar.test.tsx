import { describe, expect, it } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { mockSnapshot } from "../../mock/snapshot";
import { createAppStore, NO_MILESTONE, StoreProvider } from "../../state/store";
import type { Snapshot } from "../../types";
import { CommandPalette } from "../palette/CommandPalette";
import { PrTable } from "./PrTable";

async function setup(snapshot?: Snapshot) {
  const api = createMockApi({ latencyMs: 0, snapshot });
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

const rowIds = () => [...document.querySelectorAll<HTMLElement>("[data-pr-row]")].map((r) => r.dataset.prRow);
const bar = () => screen.queryByRole("group", { name: "Milestones" });
/** Chip labels with their counts and pressed state, e.g. "v5.1.0 4 on". */
const chips = () =>
  [...(bar() as HTMLElement).querySelectorAll("button[aria-pressed]")].map(
      (b) =>
        `${b.querySelector(".ms-chip__label")?.textContent} ${b.querySelector(".ms-chip__count")?.textContent} ${
          b.getAttribute("aria-pressed") === "true" ? "on" : "off"
        }`,
    );
const chip = (label: string) => within(bar() as HTMLElement).getByRole("button", { name: new RegExp(`^${label}, `) });
const groupMeta = (repo: string) =>
  [...document.querySelectorAll(".pr-group__head")]
    .find((h) => h.querySelector(".pr-group__title")?.textContent === repo)
    ?.querySelector(".pr-group__meta")?.textContent;

describe("Milestone switches", () => {
  it("shows one chip per milestone in the view, in version order, no milestone last", async () => {
    await setup();
    expect(chips()).toEqual(["v5.1.0 4 on", "v5.2.0 3 on", "v6.0.0 1 on", "No milestone 3 on"]);
    expect(chip("v5.1.0")).toHaveAccessibleName("v5.1.0, 4 pull requests");
    expect(chip("v5.1.0")).toHaveAttribute("title", expect.stringMatching(/Alt-click to show only this one/));
    expect(screen.queryByRole("button", { name: "Show all" })).toBeNull();
  });

  it("follows the current view", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "following" }));
    expect(chips()).toEqual(["24.1 1 on", "No milestone 4 on"]);
    // A single milestone with nothing unassigned: no choice, no row.
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dash" }));
    expect(bar()).toBeNull();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/platform" }));
    expect(chips()).toEqual(["v5.1.0 4 on", "v5.2.0 3 on", "v6.0.0 1 on", "No milestone 2 on"]);
  });

  it("is hidden when no pull request in the view has a milestone", async () => {
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    expect(bar()).toBeNull();
    expect(rowIds()).toEqual(["dashpay/dips#195"]);
  });

  it("hides and shows rows, and the group summary counts only what is shown", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    const before = groupMeta("dashpay/platform");
    expect(rowIds()).toHaveLength(11);

    await user.click(chip("v5.1.0"));
    expect(chip("v5.1.0")).toHaveAttribute("aria-pressed", "false");
    expect(store.getState().hiddenMilestones).toEqual(["v5.1.0"]);
    expect(rowIds()).toHaveLength(7);
    expect(rowIds()).not.toContain("dashpay/platform#6033");
    expect(groupMeta("dashpay/platform")).not.toBe(before);
    expect(groupMeta("dashpay/platform")).toBe("3 changes requested · 1 commented · 2 draft");
    expect(screen.getByRole("status")).toHaveTextContent("4 hidden by milestone");
    // The chip stays, with its full count, while switched off.
    expect(chips()).toEqual(["v5.1.0 4 off", "v5.2.0 3 on", "v6.0.0 1 on", "No milestone 3 on"]);

    await user.click(chip("No milestone"));
    expect(rowIds()).not.toContain("dashpay/dips#195");
    expect(document.querySelectorAll(".pr-group__title")).toHaveLength(1);
    expect(screen.getByRole("status")).toHaveTextContent("7 hidden by milestone");

    await user.click(chip("v5.1.0"));
    expect(rowIds()).toContain("dashpay/platform#6033");
    expect(screen.getByRole("status")).toHaveTextContent("3 hidden by milestone");
  });

  it("keeps the chips while the text filter matches nothing", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.click(chip("v6.0.0"));
    act(() => store.getState().setQuery("no such pull request"));
    expect(screen.getByText(/No pull requests match/)).toBeInTheDocument();
    expect(chips()).toEqual(["v5.1.0 4 on", "v5.2.0 3 on", "v6.0.0 1 off", "No milestone 3 on"]);
  });

  it("counts only the text filter's matches in the hidden line", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setQuery("dashpay/dips"));
    expect(rowIds()).toEqual(["dashpay/dips#195"]);
    await user.click(chip("v5.1.0"));
    expect(screen.queryByText(/hidden by milestone/)).toBeNull();
  });

  it("shows all again", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.click(chip("v5.1.0"));
    await user.click(chip("v5.2.0"));
    await user.click(screen.getByRole("button", { name: "Show all" }));
    expect(store.getState().hiddenMilestones).toEqual([]);
    expect(rowIds()).toHaveLength(11);
    expect(screen.queryByRole("button", { name: "Show all" })).toBeNull();
  });

  it("Alt-click shows only that milestone, and again brings the others back", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.keyboard("{Alt>}");
    await user.click(chip("v5.2.0"));
    await user.keyboard("{/Alt}");
    expect(chips()).toEqual(["v5.1.0 4 off", "v5.2.0 3 on", "v6.0.0 1 off", "No milestone 3 off"]);
    expect(rowIds()).toHaveLength(3);
    await user.keyboard("{Alt>}");
    await user.click(chip("v5.2.0"));
    await user.keyboard("{/Alt}");
    expect(store.getState().hiddenMilestones).toEqual([]);
  });

  it("says so when every pull request here is switched off, and shows all from there", async () => {
    const user = userEvent.setup();
    await setup();
    for (const label of ["v5.1.0", "v5.2.0", "v6.0.0", "No milestone"]) await user.click(chip(label));
    expect(rowIds()).toEqual([]);
    const empty = screen.getByRole("status");
    expect(empty).toHaveTextContent("All 11 pull requests here are in milestones you have switched off");
    await user.click(within(empty).getByRole("button", { name: "Show all" }));
    expect(rowIds()).toHaveLength(11);
  });

  it("keeps a switch for a milestone that is not in the current view", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "following" }));
    await user.click(chip("24.1"));
    expect(rowIds()).not.toContain("dashpay/dash#7887");
    act(() => store.getState().setView({ kind: "mine" }));
    // 24.1 is not in Mine: no chip, no "Show all", but the switch is kept.
    expect(chips().some((c) => c.startsWith("24.1"))).toBe(false);
    expect(screen.queryByRole("button", { name: "Show all" })).toBeNull();
    await user.click(chip("v6.0.0"));
    expect(store.getState().hiddenMilestones).toEqual(["24.1", "v6.0.0"]);
    act(() => store.getState().setView({ kind: "following" }));
    expect(chips()).toEqual(["24.1 1 off", "No milestone 4 on"]);
  });

  it("moves through visible rows only with the keyboard", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.click(chip("v5.1.0"));
    (document.activeElement as HTMLElement).blur();
    const seen: (string | null)[] = [];
    for (let i = 0; i < 8; i++) {
      await user.keyboard("j");
      seen.push(store.getState().focusedId);
    }
    expect(new Set(seen).size).toBe(7);
    expect(seen.every((id) => id && rowIds().includes(id))).toBe(true);
  });

  it("switches the milestone back on when the palette jumps to a hidden pull request", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.click(chip("v6.0.0"));
    await user.click(chip("No milestone"));
    expect(rowIds()).not.toContain("dashpay/platform#6002");
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("local node");
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("run an added local node");
    await user.keyboard("{Enter}");
    expect(store.getState().hiddenMilestones).toEqual([NO_MILESTONE]);
    expect(rowIds()).toContain("dashpay/platform#6002");
    expect(store.getState().expandedId).toBe("dashpay/platform#6002");
  });

  it("finds hidden pull requests in the palette", async () => {
    const user = userEvent.setup();
    await setup();
    await user.click(chip("v6.0.0"));
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("local node");
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("run an added local node");
  });

  it("remembers the switches in a new store on the same storage", async () => {
    const user = userEvent.setup();
    const { api } = await setup();
    await user.click(chip("v5.2.0"));
    const again = createAppStore(api);
    expect(again.getState().hiddenMilestones).toEqual(["v5.2.0"]);
  });

  it("shows the milestone on the row and in the detail", async () => {
    const user = userEvent.setup();
    await setup(mockSnapshot());
    const row = document.querySelector<HTMLElement>('[data-pr-row="dashpay/platform#6002"]') as HTMLElement;
    expect(row.querySelector(".pr-row__sub")?.textContent).toMatch(/^#6002 · \+100 −7 · v6\.0\.0/);
    await user.click(row);
    const detail = document.getElementById("pr-detail-dashpay-platform-6002") as HTMLElement;
    expect(within(detail).getByText("Milestone")).toBeInTheDocument();
    expect(within(detail).getByText("v6.0.0")).toBeInTheDocument();
    const dips = document.querySelector<HTMLElement>('[data-pr-row="dashpay/dips#195"]') as HTMLElement;
    expect(dips.querySelector(".pr-row__sub")?.textContent).toBe("#195 · +524 −1 · needs 1 human review");
  });
});
