import { describe, expect, it } from "vitest";
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createAppStore, NO_AREA, type AppStore } from "../../state/store";
import { PrTable } from "../table/PrTable";
import { CommandPalette } from "../palette/CommandPalette";
import { makeApi, renderShell } from "./testUtils";

const PLATFORM = "dashpay/platform";

async function setup() {
  return renderShell(makeApi(), {
    children: (
      <>
        <PrTable />
        <CommandPalette />
      </>
    ),
  });
}

const sidebar = () => screen.getByRole("complementary", { name: "Views and projects" });
const projectButton = (repo: string) =>
  within(sidebar()).getByRole("button", { name: new RegExp(`^${repo.replace("/", "\\/")}(?![a-z/-])`) });
const chevron = (repo: string) => within(sidebar()).queryByRole("button", { name: `Areas in ${repo}` });
const panel = (repo: string) => within(sidebar()).queryByRole("group", { name: `Areas in ${repo}` });
const box = (repo: string, area: string) =>
  within(panel(repo) as HTMLElement).getByRole("checkbox", { name: new RegExp(`^${area}, `) });
/** Each checkbox as "label count on|off". */
const areaList = (repo: string) =>
  within(panel(repo) as HTMLElement)
    .getAllByRole("checkbox")
    .map((b) => {
      const label = b.closest("label") as HTMLElement;
      const name = label.querySelector(".areas__label")?.textContent;
      const n = label.querySelector(".areas__n")?.textContent;
      return `${name} ${n} ${(b as HTMLInputElement).checked ? "on" : "off"}`;
    });
const rowIds = () => [...document.querySelectorAll<HTMLElement>("[data-pr-row]")].map((r) => r.dataset.prRow);
const note = () => document.querySelector(".pr-hidden-note")?.textContent ?? null;

async function openAreas(user: ReturnType<typeof userEvent.setup>, repo = PLATFORM) {
  await user.click(chevron(repo) as HTMLElement);
  return panel(repo) as HTMLElement;
}

describe("area switches in the sidebar", () => {
  it("offers the disclosure only for projects with a choice, or something switched off", async () => {
    const { store } = await setup();
    expect(chevron(PLATFORM)).not.toBeNull();
    expect(chevron("dashpay/dash-evo-tool")).not.toBeNull();
    // One area each ("Other"): nothing to choose.
    expect(chevron("dashpay/dips")).toBeNull();
    expect(chevron("dashpay/dash")).toBeNull();
    act(() => store.getState().toggleArea("dashpay/dips", NO_AREA));
    expect(chevron("dashpay/dips")).not.toBeNull();
  });

  it("lists the project's areas with counts when opened", async () => {
    const user = userEvent.setup();
    await setup();
    expect(chevron(PLATFORM)).toHaveAttribute("aria-expanded", "false");
    expect(panel(PLATFORM)).toBeNull();
    await openAreas(user);
    expect(chevron(PLATFORM)).toHaveAttribute("aria-expanded", "true");
    expect(areaList(PLATFORM)).toEqual([
      "dashmate 1 on",
      "drive-abci 1 on",
      "kotlin-example-app 1 on",
      "platform 2 on",
      "platform-wallet 1 on",
      "sdk 3 on",
      "swift-sdk 1 on",
      "Other 1 on",
    ]);
    expect(box(PLATFORM, "sdk")).toHaveAccessibleName("sdk, 3 pull requests");
    // The view did not change.
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Mine");
  });

  it("hides an unticked area's pull requests in Mine and in the project view", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await openAreas(user);
    await user.click(box(PLATFORM, "sdk"));
    expect(store.getState().hiddenAreas).toEqual({ [PLATFORM]: ["sdk"] });
    expect(rowIds()).not.toContain("dashpay/platform#6034");
    expect(rowIds()).not.toContain("dashpay/platform#5967");
    expect(rowIds()).toHaveLength(9);
    expect(note()).toBe("2 hidden by area");
    // Sidebar counts stay totals.
    expect(within(sidebar()).getByRole("button", { name: /^Mine/ })).toHaveTextContent("11");
    expect(projectButton(PLATFORM)).toHaveTextContent(/10 pull requests$/);

    await user.click(projectButton(PLATFORM));
    expect(store.getState().view).toEqual({ kind: "project", repo: PLATFORM });
    expect(rowIds()).toHaveLength(8);
    expect(rowIds()).not.toContain("dashpay/platform#6034");
    expect(note()).toBe("2 hidden by area");

    act(() => store.getState().setView({ kind: "needsMe" }));
    expect(rowIds()).not.toContain("dashpay/platform#6034");
  });

  it("keeps a pull request with several areas while one of them is on", async () => {
    const user = userEvent.setup();
    await setup();
    await openAreas(user);
    await user.click(box(PLATFORM, "sdk"));
    expect(rowIds()).toContain("dashpay/platform#6022");
    await user.click(box(PLATFORM, "drive-abci"));
    expect(rowIds()).not.toContain("dashpay/platform#6022");
  });

  it("marks a filtered project on its row, with what is hidden in the tooltip", async () => {
    const user = userEvent.setup();
    await setup();
    expect(projectButton(PLATFORM).querySelector(".nav-filter")).toBeNull();
    await openAreas(user);
    await user.click(box(PLATFORM, "sdk"));
    await user.click(box(PLATFORM, "Other"));
    // Closed again, the row still says it is filtered.
    await user.click(chevron(PLATFORM) as HTMLElement);
    expect(panel(PLATFORM)).toBeNull();
    const marker = projectButton(PLATFORM).querySelector(".nav-filter") as HTMLElement;
    expect(marker).toHaveTextContent("2 off");
    expect(marker).toHaveAttribute("title", "Areas switched off: sdk, Other. 3 of 10 listed pull requests hidden.");
    expect(projectButton(PLATFORM)).toHaveAccessibleName(/2 areas switched off/);
  });

  it("switches every area off with None and back on with All", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    const p = await openAreas(user);
    const all = within(p).getByRole("button", { name: `Show all areas in ${PLATFORM}` });
    const none = within(p).getByRole("button", { name: `Hide all areas in ${PLATFORM}` });
    expect(all).toBeDisabled();
    await user.click(none);
    expect(store.getState().hiddenAreas[PLATFORM]).toHaveLength(8);
    expect(areaList(PLATFORM).every((a) => a.endsWith(" off"))).toBe(true);
    expect(none).toBeDisabled();
    expect(p).toHaveTextContent("Every area is off, so none of this project’s pull requests are listed.");
    expect(rowIds()).toEqual(["dashpay/dips#195"]);
    await user.click(all);
    expect(store.getState().hiddenAreas).toEqual({});
    expect(rowIds()).toHaveLength(11);
  });

  it("keeps a switch for an area no longer listed, and All clears it", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().toggleArea("dashpay/dash", "gone"));
    // dashpay/dash has a single area, but something to undo: the disclosure shows.
    expect(projectButton("dashpay/dash")).toHaveTextContent("1 off");
    const p = await openAreas(user, "dashpay/dash");
    expect(p).toHaveTextContent("Also off, with nothing listed now: gone");
    expect(areaList("dashpay/dash")).toEqual(["Other 1 on"]);
    await user.click(within(p).getByRole("button", { name: "Show all areas in dashpay/dash" }));
    expect(store.getState().hiddenAreas).toEqual({});
    expect(chevron("dashpay/dash")).toBeNull();
  });

  it("works from the keyboard", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setFocused("dashpay/platform#6033"));
    chevron(PLATFORM)?.focus();
    await user.keyboard("{Enter}");
    expect(panel(PLATFORM)).not.toBeNull();
    box(PLATFORM, "sdk").focus();
    await user.keyboard(" ");
    expect(box(PLATFORM, "sdk")).not.toBeChecked();
    expect(store.getState().hiddenAreas).toEqual({ [PLATFORM]: ["sdk"] });
    // Space toggled the checkbox, not the focused row's detail.
    expect(store.getState().expandedId).toBeNull();
    await user.tab();
    expect(document.activeElement).toBe(box(PLATFORM, "swift-sdk"));
    await user.keyboard(" ");
    expect(store.getState().hiddenAreas[PLATFORM]).toEqual(["sdk", "swift-sdk"]);
    await user.keyboard("{Escape}");
    expect(panel(PLATFORM)).toBeNull();
    expect(document.activeElement).toBe(chevron(PLATFORM));
  });

  it("still selects the project from its name, and the chevron does not", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await openAreas(user);
    expect(store.getState().view).toEqual({ kind: "mine" });
    await user.click(projectButton(PLATFORM));
    expect(store.getState().view).toEqual({ kind: "project", repo: PLATFORM });
    expect(panel(PLATFORM)).not.toBeNull();
  });

  it("remembers which lists are open", async () => {
    const user = userEvent.setup();
    await setup();
    await openAreas(user);
    expect(JSON.parse(localStorage.getItem("pr-status.sidebar.areas-open.v1") ?? "[]")).toEqual([PLATFORM]);
  });
});

describe("area switches in the list", () => {
  const offMilestone = (store: AppStore, m: string) => act(() => store.getState().toggleMilestone(m));
  const offArea = (store: AppStore, a: string, repo = PLATFORM) => act(() => store.getState().toggleArea(repo, a));

  it("counts hidden pull requests by cause, once each", async () => {
    const { store } = await setup();
    offMilestone(store, "v5.1.0");
    offArea(store, "sdk");
    // v5.1.0 hides #6033 #6038 #6034 #5750; sdk hides #6034 (counted under
    // milestone) and #5967.
    expect(note()).toBe("4 hidden by milestone · 1 hidden by area");
    expect(rowIds()).toHaveLength(6);
    expect(rowIds()).not.toContain("dashpay/platform#5967");
  });

  it("says so when areas hide everything here, and shows all from there", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: PLATFORM }));
    const p = await openAreas(user);
    await user.click(within(p).getByRole("button", { name: `Hide all areas in ${PLATFORM}` }));
    const empty = within(screen.getByRole("main")).getByRole("status");
    expect(empty).toHaveTextContent(`All 10 pull requests here are in areas of ${PLATFORM} you have switched off`);
    await user.click(within(empty).getByRole("button", { name: "Show all" }));
    expect(rowIds()).toHaveLength(10);
  });

  it("names both causes when milestones and areas together hide everything", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setView({ kind: "project", repo: PLATFORM }));
    for (const m of ["v5.1.0", "v5.2.0", "v6.0.0"]) offMilestone(store, m);
    offArea(store, "sdk");
    offArea(store, NO_AREA);
    expect(rowIds()).toEqual([]);
    const empty = within(screen.getByRole("main")).getByRole("status");
    expect(empty).toHaveTextContent(
      `All 10 pull requests here are in milestones or areas of ${PLATFORM} you have switched off`,
    );
    expect(empty).toHaveTextContent("8 hidden by milestone · 2 hidden by area");
    await user.click(within(empty).getByRole("button", { name: "Show all areas" }));
    expect(store.getState().hiddenAreas).toEqual({});
    expect([...rowIds()].sort()).toEqual(["dashpay/platform#5749", "dashpay/platform#5967"]);
    offArea(store, "sdk");
    offArea(store, NO_AREA);
    await user.click(
      within(within(screen.getByRole("main")).getByRole("status")).getByRole("button", { name: "Show all milestones" }),
    );
    expect(store.getState().hiddenMilestones).toEqual([]);
  });

  it("moves through visible rows only with the keyboard", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    offArea(store, "platform");
    const seen = new Set<string | null>();
    for (let i = 0; i < 12; i++) {
      await user.keyboard("j");
      seen.add(store.getState().focusedId);
    }
    expect(seen.size).toBe(9);
    expect([...seen].every((id) => id && rowIds().includes(id))).toBe(true);
  });

  it("switches the pull request's areas back on when the palette jumps to it", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    offArea(store, "sdk");
    offArea(store, "drive-abci");
    offArea(store, "platform");
    expect(rowIds()).not.toContain("dashpay/platform#6022");
    await user.keyboard("{Control>}k{/Control}");
    await user.keyboard("document history");
    expect(screen.getAllByRole("option")[0]).toHaveTextContent("read document history");
    await user.keyboard("{Enter}");
    // Only the areas this pull request names come back.
    expect(store.getState().hiddenAreas).toEqual({ [PLATFORM]: ["platform"] });
    expect(rowIds()).toContain("dashpay/platform#6022");
    expect(store.getState().expandedId).toBe("dashpay/platform#6022");
  });

  it("remembers the switches in a new store on the same storage", async () => {
    const user = userEvent.setup();
    const { api } = await setup();
    await openAreas(user);
    await user.click(box(PLATFORM, "dashmate"));
    expect(createAppStore(api).getState().hiddenAreas).toEqual({ [PLATFORM]: ["dashmate"] });
  });
});
