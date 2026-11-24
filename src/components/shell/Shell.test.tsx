import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { mockSnapshot } from "../../mock/snapshot";
import { prsForView, type View } from "../../lib/select";
import type { Snapshot } from "../../types";
import { makeApi, renderShell } from "./testUtils";

const prIdsInView = (snapshot: Snapshot | null, view: View) => prsForView(snapshot, view).map((pr) => pr.id);

const sidebar = () => screen.getByRole("complementary", { name: "Views and projects" });
const navButton = (name: RegExp) => within(sidebar()).getByRole("button", { name });

describe("sidebar", () => {
  it("shows view and project counts from the snapshot", async () => {
    await renderShell();
    expect(navButton(/^Mine/)).toHaveTextContent("11");
    expect(navButton(/^Following/)).toHaveTextContent("5");
    expect(navButton(/^Needs me/)).toHaveTextContent(/\d/);
    const platform = navButton(/^dashpay\/platform/);
    expect(platform).toHaveAttribute("title", expect.stringContaining("137 open"));
    // Projects appear in config order.
    const names = within(sidebar())
      .getAllByRole("button", { name: /^dashpay\// })
      .map((b) => b.textContent?.replace(/\d+ pull requests$/, ""));
    expect(names).toEqual(["dashpay/platform", "dashpay/dips", "dashpay/dash-evo-tool", "dashpay/dash"]);
  });

  it("switches views and marks the active one", async () => {
    const user = userEvent.setup();
    const { store } = await renderShell();
    expect(navButton(/^Mine/)).toHaveAttribute("aria-current", "page");
    await user.click(navButton(/^Following/));
    expect(store.getState().view).toEqual({ kind: "following" });
    expect(navButton(/^Following/)).toHaveAttribute("aria-current", "page");
    expect(navButton(/^Mine/)).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Following");

    await user.click(navButton(/^dashpay\/dips/));
    expect(store.getState().view).toEqual({ kind: "project", repo: "dashpay/dips" });
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("dashpay/dips");
  });

  it("marks a project that failed to load", async () => {
    const snapshot = mockSnapshot();
    snapshot.projects[1] = { ...snapshot.projects[1], error: "Repository not found" };
    await renderShell(makeApi({ snapshot }));
    expect(navButton(/^dashpay\/dips/)).toHaveTextContent("Could not load: Repository not found");
  });

  it("shows the review bot's status", async () => {
    const snapshot = mockSnapshot();
    snapshot.feed = { ...snapshot.feed, degraded: true };
    await renderShell(makeApi({ snapshot }));
    const feed = screen.getByRole("region", { name: "PastaClaw" });
    expect(feed).toHaveTextContent("4 running · 10 queued");
    expect(feed).toHaveTextContent("median review 19 min");
    expect(feed).toHaveTextContent("Running on stand-in models");
  });

  it("warns when the review feed is down", async () => {
    const snapshot = mockSnapshot();
    snapshot.feed = { ...snapshot.feed, ok: false, error: "Feed returned 503" };
    await renderShell(makeApi({ snapshot }));
    expect(screen.getByRole("region", { name: "PastaClaw" })).toHaveTextContent("Feed returned 503");
  });
});

describe("header", () => {
  it("shows when it last synced and syncs on click", async () => {
    const user = userEvent.setup();
    const { api } = await renderShell();
    const refresh = vi.spyOn(api, "refresh");
    const sync = screen.getByRole("button", { name: /^Synced \d+ s ago/ });
    await user.click(sync);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it("shows Syncing… while a sync runs", async () => {
    const { store } = await renderShell();
    act(() => store.setState({ syncing: true }));
    expect(screen.getByRole("button", { name: "Syncing…" })).toBeInTheDocument();
  });

  it("opens the palette from the quick-filter button", async () => {
    const user = userEvent.setup();
    const { store } = await renderShell();
    await user.click(screen.getByRole("button", { name: /Quick filter/ }));
    expect(store.getState().paletteOpen).toBe(true);
  });
});

describe("app-level states", () => {
  it("shows a table skeleton while the first load runs", async () => {
    await renderShell(makeApi(), { init: false });
    expect(screen.getByText("Loading pull requests…")).toBeInTheDocument();
    expect(screen.queryByTestId("table")).not.toBeInTheDocument();
  });

  it("shows the load error with Try again", async () => {
    const user = userEvent.setup();
    let fail = true;
    const base = makeApi();
    const api = makeApi({}, {
      getSnapshot: () => (fail ? Promise.reject("Backend did not answer") : base.getSnapshot()),
    });
    await renderShell(api);
    expect(screen.getByRole("heading", { name: /Could not load your pull requests/ })).toBeInTheDocument();
    expect(screen.getByText("Backend did not answer")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByTestId("table")).toBeInTheDocument();
  });

  it("asks the user to sign in when there is no token", async () => {
    const user = userEvent.setup();
    const snapshot = { ...mockSnapshot(), viewer: null, signInRequired: true };
    snapshot.github = { ok: false, lastSuccessAt: null, error: "No GitHub token found" };
    const { api } = await renderShell(makeApi({ snapshot }));
    expect(screen.getByRole("heading", { name: "Sign in to GitHub" })).toBeInTheDocument();
    expect(screen.getByText("gh auth login")).toBeInTheDocument();
    expect(screen.getByText("GITHUB_TOKEN")).toBeInTheDocument();
    expect(screen.getByText("No GitHub token found")).toBeInTheDocument();
    const refresh = vi.spyOn(api, "refresh");
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(refresh).toHaveBeenCalled();
  });

  it("does not ask an offline user to sign in", async () => {
    // GitHub could not be reached at startup: no viewer yet, but the token may be fine.
    const snapshot = { ...mockSnapshot(), viewer: null, signInRequired: false };
    snapshot.github = { ok: false, lastSuccessAt: null, error: "network error: the request timed out" };
    await renderShell(makeApi({ snapshot }));
    expect(screen.queryByRole("heading", { name: "Sign in to GitHub" })).not.toBeInTheDocument();
    expect(screen.getByText(/network error: the request timed out/)).toBeInTheDocument();
    expect(screen.getByTestId("table")).toBeInTheDocument();
  });

  it("invites the user to add a project when there are none", async () => {
    const user = userEvent.setup();
    const snapshot = { ...mockSnapshot(), projects: [] };
    await renderShell(makeApi({ snapshot, config: { projects: [], followed: [], contributors: [] } }));
    expect(screen.getByRole("heading", { name: "Add a project to see its pull requests" })).toBeInTheDocument();
    await user.click(within(screen.getByRole("main")).getByRole("button", { name: "+ Add project" }));
    expect(screen.getByRole("dialog", { name: "Add project" })).toBeInTheDocument();
  });

  it("renders the table with a banner when GitHub is failing", async () => {
    const user = userEvent.setup();
    const snapshot = mockSnapshot();
    snapshot.github = { ok: false, lastSuccessAt: new Date(Date.now() - 180_000).toISOString(), error: "Rate limited" };
    const { api } = await renderShell(makeApi({ snapshot }));
    expect(screen.getByTestId("table")).toBeInTheDocument();
    const banner = screen.getByRole("status");
    expect(banner).toHaveTextContent("Rate limited");
    expect(banner).toHaveTextContent("Last synced 3 min ago");
    const refresh = vi.spyOn(api, "refresh");
    await user.click(within(banner).getByRole("button", { name: "Retry" }));
    expect(refresh).toHaveBeenCalled();
  });

  it("renders the table without a banner when all is well", async () => {
    await renderShell();
    expect(screen.getByTestId("table")).toBeInTheDocument();
    expect(screen.queryByText(/Could not reach GitHub/)).not.toBeInTheDocument();
  });
});

describe("shortcuts", () => {
  const press = (key: string, target: Element = document.body) =>
    fireEvent.keyDown(target, { key, ctrlKey: true, bubbles: true });

  it("switches views with Ctrl 1, 2 and 3", async () => {
    const { store } = await renderShell();
    press("2");
    expect(store.getState().view).toEqual({ kind: "following" });
    press("3");
    expect(store.getState().view).toEqual({ kind: "needsMe" });
    press("1");
    expect(store.getState().view).toEqual({ kind: "mine" });
  });

  it("refreshes with Ctrl R and opens Add project with Ctrl N", async () => {
    const { api } = await renderShell();
    const refresh = vi.spyOn(api, "refresh");
    press("r");
    expect(refresh).toHaveBeenCalledTimes(1);
    press("n");
    expect(screen.getByRole("dialog", { name: "Add project" })).toBeInTheDocument();
  });

  it("ignores shortcuts while typing or while a dialog is open", async () => {
    const { store } = await renderShell();
    press("n");
    const field = screen.getByRole("textbox", { name: "Repository" });
    press("2", field);
    expect(store.getState().view).toEqual({ kind: "mine" });
    fireEvent.keyDown(field, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    press("2");
    expect(store.getState().view).toEqual({ kind: "following" });
  });

  it("does not take the palette's Ctrl K", async () => {
    const { store } = await renderShell();
    press("k");
    expect(store.getState().paletteOpen).toBe(false);
  });
});

describe("running example apps", () => {
  it("lists Local clone… in each project's menu", async () => {
    const user = userEvent.setup();
    await renderShell();
    await user.click(screen.getByRole("button", { name: "Actions for dashpay/dips" }));
    const items = screen.getAllByRole("menuitem").map((i) => i.textContent);
    expect(items).toEqual(["Browse open PRs", "Local clone…", "Remove…"]);
  });

  it("shows no run status or panel before anything has run", async () => {
    await renderShell();
    expect(screen.queryByRole("button", { name: /^Running on|run failed|^Opened on/ })).toBeNull();
    expect(document.getElementById("run-panel")).toBeNull();
  });
});

describe("tracked contributors", () => {
  it("leave the user's own views and the sidebar counts as they were", async () => {
    const { store } = await renderShell();
    const counts = () =>
      within(sidebar())
        .getAllByRole("button", { name: /pull requests$/ })
        .map((b) => b.textContent);
    const views: View[] = [
      { kind: "mine" },
      { kind: "following" },
      { kind: "needsMe" },
      ...store.getState().config!.projects.map((repo): View => ({ kind: "project", repo })),
    ];
    const lists = () => views.map((view) => prIdsInView(store.getState().snapshot, view));
    const before = { counts: counts(), lists: lists() };

    await act(async () => {
      await store.getState().trackContributor("OrbitWren");
      await store.getState().trackContributor("kestrel-dev");
    });
    expect(store.getState().view).toEqual({ kind: "contributor", login: "kestrel-dev" });
    // The snapshot holds their pull requests now...
    expect(prIdsInView(store.getState().snapshot, { kind: "contributor", login: "OrbitWren" })).toHaveLength(3);
    // ...and nothing the user already saw changed.
    expect(counts()).toEqual(before.counts);
    expect(lists()).toEqual(before.lists);
    // Nothing in the sidebar is marked while a contributor view shows.
    act(() => store.getState().setView({ kind: "contributor", login: "OrbitWren" }));
    expect(within(sidebar()).queryAllByRole("button", { current: "page" })).toEqual([]);
  });
});
