// The "By subcrate" box beside Sort, and the sub-groups it adds inside each
// project group.

import { describe, expect, it } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { createAppStore, NO_AREA, StoreProvider } from "../../state/store";
import { PrTable } from "./PrTable";

const PLATFORM = "dashpay/platform";

async function setup(prefs?: object) {
  if (prefs) localStorage.setItem("pr-status.prefs.v1", JSON.stringify(prefs));
  const api = createMockApi({ latencyMs: 0 });
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
    </StoreProvider>,
  );
  return { store, api };
}

const box = () => screen.getByRole("checkbox", { name: "By subcrate" });
const rows = () => [...document.querySelectorAll<HTMLElement>("[data-pr-row]")];
const rowIds = () => rows().map((r) => r.dataset.prRow);
const row = (id: string) => rows().find((r) => r.dataset.prRow === id) as HTMLElement;
/** Sub-group headings by accessible name ("sdk, 3 pull requests"). */
const subHeads = () => screen.queryAllByRole("heading", { level: 3 }).map((h) => h.getAttribute("aria-label"));
/** The project section a heading's text belongs to. */
const projectSection = (repo: string) => screen.getByRole("region", { name: new RegExp(`^${repo}`) });
const note = () => document.querySelector(".pr-hidden-note")?.textContent ?? null;

const PLATFORM_SUBS = [
  "swift-sdk, 1 pull request",
  "platform, 2 pull requests",
  "sdk, 3 pull requests",
  "kotlin-example-app, 1 pull request",
  "platform-wallet, 1 pull request",
  "dashmate, 1 pull request",
  "Other, 1 pull request",
];

describe("By subcrate box", () => {
  it("sits after the Sort control, unticked by default, with no sub-groups", async () => {
    await setup();
    const toolbar = document.querySelector(".pr-toolbar") as HTMLElement;
    const sort = within(toolbar).getByRole("group", { name: "Sort" });
    expect(sort.compareDocumentPosition(box()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(box()).not.toBeChecked();
    expect(box()).toBeEnabled();
    expect(subHeads()).toEqual([]);
  });

  it("splits each project into subcrates when ticked, and remembers it", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await user.click(box());
    expect(store.getState().bySubcrate).toBe(true);
    expect(JSON.parse(localStorage.getItem("pr-status.prefs.v1") ?? "{}").bySubcrate).toBe(true);
    // The one dips PR has no scope: it still gets its "Other" heading.
    expect(subHeads()).toEqual([...PLATFORM_SUBS, "Other, 1 pull request"]);
    await user.click(box());
    expect(subHeads()).toEqual([]);
    expect(JSON.parse(localStorage.getItem("pr-status.prefs.v1") ?? "{}").bySubcrate).toBe(false);
  });

  it("puts sub-groups inside their project, under its unchanged summary", async () => {
    await setup({ bySubcrate: true });
    const platform = projectSection(PLATFORM);
    expect(within(platform).getByText("1 reviewing · 4 changes requested · 1 commented · 2 approved · 2 draft")).toBeInTheDocument();
    const sdk = within(platform).getByRole("region", { name: "sdk, 3 pull requests" });
    expect(within(sdk).getByRole("heading", { level: 3 })).toHaveTextContent("sdk3");
    expect(within(sdk).getAllByRole("button", { expanded: false }).map((r) => r.dataset.prRow)).toEqual([
      "dashpay/platform#6022",
      "dashpay/platform#6034",
      "dashpay/platform#5967",
    ]);
    // #6022, "fix(sdk, drive-abci): ...", is listed once, under its first area.
    expect(rowIds().filter((id) => id === "dashpay/platform#6022")).toHaveLength(1);
    expect(screen.queryByRole("region", { name: /^drive-abci,/ })).toBeNull();
  });

  it("is disabled with a hint while grouping by status, and leaves status groups alone", async () => {
    const user = userEvent.setup();
    const { store } = await setup({ bySubcrate: true });
    await user.click(screen.getByRole("button", { name: "Status" }));
    expect(box()).toBeDisabled();
    expect(box()).toBeChecked();
    expect(box()).toHaveAccessibleDescription("Group by project to split by subcrate");
    expect(box().closest("label")).toHaveAttribute("title", "Group by project to split by subcrate");
    expect(subHeads()).toEqual([]);
    await user.click(box());
    expect(store.getState().bySubcrate).toBe(true);
    // Back to Project: the box works again and the split returns.
    await user.click(screen.getByRole("button", { name: "Project" }));
    expect(box()).toBeEnabled();
    expect(box()).not.toHaveAccessibleDescription();
    expect(subHeads()).toHaveLength(PLATFORM_SUBS.length + 1);
  });

  it("orders sub-groups and their rows by the chosen sort", async () => {
    const user = userEvent.setup();
    await setup({ bySubcrate: true });
    // Finishes first: the running review (swift-sdk) leads.
    expect(rowIds()[0]).toBe("dashpay/platform#6033");
    expect(subHeads()[0]).toBe("swift-sdk, 1 pull request");
    await user.click(screen.getByRole("button", { name: "Updated" }));
    expect(rowIds()[0]).toBe("dashpay/platform#6033");
    // Rows of one sub-group stay together, in sort order.
    const platformRows = rowIds().filter((id) => ["dashpay/platform#6038", "dashpay/platform#5750"].includes(id!));
    expect(platformRows).toEqual(["dashpay/platform#6038", "dashpay/platform#5750"]);
    const ids = rowIds();
    expect(ids.indexOf("dashpay/platform#5750") - ids.indexOf("dashpay/platform#6038")).toBe(1);
  });

  it("moves the keyboard cursor through rows in the order shown, across sub-groups and projects", async () => {
    const user = userEvent.setup();
    const { store } = await setup({ bySubcrate: true });
    const ids = rowIds() as string[];
    await user.keyboard("{ArrowDown}");
    expect(store.getState().focusedId).toBe(ids[0]);
    await user.keyboard("jj");
    expect(store.getState().focusedId).toBe(ids[2]);
    expect(row(ids[2])).toHaveFocus();
    // ids[1] and ids[2] are the two "platform" rows; the next one starts "sdk".
    await user.keyboard("j");
    expect(store.getState().focusedId).toBe("dashpay/platform#6022");
    await user.keyboard("{Enter}");
    expect(store.getState().expandedId).toBe("dashpay/platform#6022");
    expect(row("dashpay/platform#6022")).toHaveAttribute("aria-expanded", "true");
    await user.keyboard("{Escape}");
    for (let i = 0; i < 20; i++) await user.keyboard("j");
    expect(store.getState().focusedId).toBe("dashpay/dips#195");
  });

  it("keeps working with the text filter, the drafts switch and the area switches", async () => {
    const { store } = await setup({ bySubcrate: true });
    // Text filter: only matching rows, only their sub-groups.
    fireEvent.change(screen.getByPlaceholderText("Filter pull requests"), { target: { value: "sdk" } });
    expect(subHeads().every((h) => !h?.startsWith("dashmate"))).toBe(true);
    expect(subHeads()).toContain("sdk, 3 pull requests");
    fireEvent.change(screen.getByPlaceholderText("Filter pull requests"), { target: { value: "" } });

    // Drafts hidden: #5967 (sdk) and #5968 (platform-wallet) and #6002 (dashmate) go.
    act(() => store.getState().setShowDrafts(false));
    expect(subHeads()).toContain("sdk, 2 pull requests");
    expect(subHeads().some((h) => h?.startsWith("platform-wallet,"))).toBe(false);
    expect(note()).toMatch(/hidden/);
    act(() => store.getState().setShowDrafts(true));

    // An area switched off: its sub-group goes, and the two-area PR moves to its other area.
    act(() => store.getState().toggleArea(PLATFORM, "sdk"));
    expect(subHeads().some((h) => h?.startsWith("sdk,"))).toBe(false);
    expect(subHeads()).toContain("drive-abci, 1 pull request");
    const driveAbci = screen.getByRole("region", { name: "drive-abci, 1 pull request" });
    expect(within(driveAbci).getAllByRole("button", { expanded: false })[0].dataset.prRow).toBe("dashpay/platform#6022");
    expect(note()).toMatch(/hidden by area/);
    act(() => store.getState().toggleArea(PLATFORM, NO_AREA));
    expect(subHeads().some((h) => h?.startsWith("Other,") && h.endsWith("1 pull request"))).toBe(true); // dips' Other stays
  });

  it("splits every view: Following, Needs me, a project, a contributor", async () => {
    const { store } = await setup({ bySubcrate: true });
    act(() => store.getState().setView({ kind: "following" }));
    expect(subHeads()).toEqual([
      "ci, 1 pull request",
      "tokens, 1 pull request",
      "wallet, 1 pull request",
      "contracts, 1 pull request",
      "Other, 1 pull request",
    ]);
    act(() => store.getState().setView({ kind: "needsMe" }));
    expect(subHeads().length).toBeGreaterThan(0);
    expect(rows()).toHaveLength(subHeads().reduce((n, h) => n + Number(/, (\d+) pull/.exec(h ?? "")?.[1]), 0));
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    expect(subHeads()).toEqual(["Other, 1 pull request"]);
    await act(() => store.getState().trackContributor("OrbitWren"));
    await waitFor(() => expect(rows().length).toBeGreaterThan(0));
    expect(subHeads()).toContain("sdk, 1 pull request");
  });

  it("shows the empty states as before", async () => {
    const { store } = await setup({ bySubcrate: true });
    fireEvent.change(screen.getByPlaceholderText("Filter pull requests"), { target: { value: "zzzz-nothing" } });
    expect(screen.getByText("No pull requests match “zzzz-nothing”")).toBeInTheDocument();
    expect(subHeads()).toEqual([]);
    fireEvent.change(screen.getByPlaceholderText("Filter pull requests"), { target: { value: "" } });
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dips" }));
    act(() => store.getState().toggleArea("dashpay/dips", NO_AREA));
    expect(screen.getByText(/switched off/)).toBeInTheDocument();
    expect(subHeads()).toEqual([]);
  });
});
