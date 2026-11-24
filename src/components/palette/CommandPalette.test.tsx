import { describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { mockConfig } from "../../mock/snapshot";
import { createAppStore, StoreProvider } from "../../state/store";
import { PrTable } from "../table/PrTable";
import { CommandPalette } from "./CommandPalette";

async function setup(contributors: string[] = []) {
  const api = createMockApi({ latencyMs: 0, config: { ...mockConfig(), contributors } });
  const refresh = vi.spyOn(api, "refresh");
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <button type="button">Somewhere else</button>
      <PrTable />
      <CommandPalette />
    </StoreProvider>,
  );
  return { store, refresh };
}

// jsdom reports a non-Mac platform, so the shortcut is Ctrl K.
const togglePalette = (user: ReturnType<typeof userEvent.setup>) => user.keyboard("{Control>}k{/Control}");

describe("CommandPalette", () => {
  it("renders nothing while closed and toggles with Ctrl K", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    expect(screen.queryByRole("dialog")).toBeNull();
    await togglePalette(user);
    expect(store.getState().paletteOpen).toBe(true);
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("listbox", { name: "Results" })).toBeInTheDocument();
    expect(within(dialog).getByRole("combobox")).toHaveFocus();
    await togglePalette(user);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("lists commands for every view and project", async () => {
    const user = userEvent.setup();
    await setup();
    await togglePalette(user);
    for (const name of ["Go to Mine", "Go to Following", "Go to Needs me", "Go to dashpay/dips", "Refresh"]) {
      expect(screen.getByRole("option", { name: new RegExp(name) })).toBeInTheDocument();
    }
  });

  it("finds PRs in every project and jumps to one outside the current view", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    act(() => store.getState().setQuery("something"));
    await togglePalette(user);
    // #1161 is someone else's PR: not in Mine.
    await user.keyboard("release signing");
    const options = screen.getAllByRole("option");
    expect(options[0]).toHaveTextContent("fix(ci): isolate release signing");
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{Enter}");

    const state = store.getState();
    expect(state.paletteOpen).toBe(false);
    expect(state.view).toEqual({ kind: "project", repo: "dashpay/dash-evo-tool" });
    expect(state.query).toBe("");
    expect(state.expandedId).toBe("dashpay/dash-evo-tool#1161");
    expect(screen.getByRole("region", { name: "Details for #1161" })).toBeInTheDocument();
    const row = document.querySelector<HTMLElement>('[data-pr-row="dashpay/dash-evo-tool#1161"]');
    expect(row).toHaveFocus();
  });

  it("keeps the current view when it already holds the PR", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await togglePalette(user);
    await user.keyboard("6034{Enter}");
    expect(store.getState().view).toEqual({ kind: "mine" });
    expect(store.getState().expandedId).toBe("dashpay/platform#6034");
  });

  it("moves with the arrow keys and runs a command", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    await togglePalette(user);
    await user.keyboard("go to");
    const options = screen.getAllByRole("option");
    expect(options[0]).toHaveTextContent("Go to Mine");
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(screen.getAllByRole("option")[2]).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowUp}{Enter}");
    expect(store.getState().view).toEqual({ kind: "following" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("runs Refresh", async () => {
    const user = userEvent.setup();
    const { refresh } = await setup();
    await togglePalette(user);
    await user.keyboard("refresh{Enter}");
    expect(refresh).toHaveBeenCalled();
  });

  it("closes on Esc and on a backdrop click, returning focus", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    const elsewhere = screen.getByRole("button", { name: "Somewhere else" });
    elsewhere.focus();
    await togglePalette(user);
    await user.keyboard("{Escape}");
    expect(store.getState().paletteOpen).toBe(false);
    expect(elsewhere).toHaveFocus();
    // Esc in the palette does not reach the list.
    expect(store.getState().expandedId).toBeNull();

    await togglePalette(user);
    await user.click(document.querySelector(".palette-backdrop") as HTMLElement);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(elsewhere).toHaveFocus();
  });

  it("says when nothing matches", async () => {
    const user = userEvent.setup();
    await setup();
    await togglePalette(user);
    await user.keyboard("qqqq");
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("Nothing matches “qqqq”")).toBeInTheDocument();
  });

  describe("running an example app", () => {
    it("offers to run the open PR on each platform its project has, and opens the run dialog", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      await act(async () => {
        await store.getState().loadRunTargets();
      });
      act(() => store.getState().toggleExpanded("dashpay/platform#6033"));
      await togglePalette(user);
      expect(screen.getByRole("option", { name: /Run #6033 on iOS/ })).toBeInTheDocument();
      await user.keyboard("android");
      const option = screen.getByRole("option", { name: /Run #6033 on Android/ });
      expect(screen.queryByRole("option", { name: /Run #6033 on iOS/ })).toBeNull();
      await user.click(option);
      expect(store.getState().paletteOpen).toBe(false);
      const dialog = await screen.findByRole("dialog", { name: "Run #6033" });
      expect(await within(dialog).findByRole("button", { name: "Android" })).toHaveAttribute("aria-pressed", "true");
      await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("uses the focused PR when none is open", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      await act(async () => {
        await store.getState().loadRunTargets();
      });
      act(() => store.getState().setFocused("dashpay/platform#6034"));
      await togglePalette(user);
      expect(screen.getByRole("option", { name: /Run #6034 on iOS/ })).toBeInTheDocument();
    });

    it("offers nothing to run without a PR, or for a project without example apps", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      await act(async () => {
        await store.getState().loadRunTargets();
      });
      await togglePalette(user);
      expect(screen.queryByRole("option", { name: /^Run #/ })).toBeNull();
      await togglePalette(user);
      act(() => store.getState().setFocused("dashpay/dash-evo-tool#1213"));
      await togglePalette(user);
      expect(screen.queryByRole("option", { name: /^Run #/ })).toBeNull();
    });
  });

  describe("posting the self-review", () => {
    it("offers it for the open PR when the gate wants it, and opens the confirmation", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      act(() => store.getState().toggleExpanded("dashpay/platform#6038"));
      await togglePalette(user);
      const option = screen.getByRole("option", { name: /Post \/self-reviewed on #6038/ });
      expect(within(option).getByText("/self-reviewed")).toHaveClass("mono");
      // It leads the commands, as the thing to do now.
      expect(screen.getAllByRole("option")[0]).toBe(option);
      await user.keyboard("{Enter}");
      expect(store.getState().paletteOpen).toBe(false);
      const dialog = await screen.findByRole("dialog", { name: "Post /self-reviewed on #6038?" });
      expect(await within(dialog).findByRole("button", { name: "Cancel" })).toHaveFocus();
      expect(dialog.querySelector("pre")?.textContent).toMatch(/^\/self-reviewed [0-9a-f]{40}$/);
      await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("uses the focused PR when none is open, says again once posted, and finds it by typing", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      act(() => store.getState().setFocused("dashpay/platform#6033"));
      await togglePalette(user);
      expect(screen.getByRole("option", { name: /Post \/self-reviewed on #6033/ })).toBeInTheDocument();
      await togglePalette(user);

      await act(async () => {
        await store.getState().postSelfReview("dashpay/platform#6038", "5cc1bd0123d8a0f0c5a56ca6c43ec69f7e1b2c3d");
      });
      act(() => store.getState().setFocused("dashpay/platform#6038"));
      await togglePalette(user);
      await user.keyboard("self-reviewed");
      expect(screen.getByRole("option", { name: /Post \/self-reviewed again on #6038/ })).toBeInTheDocument();
    });

    it("offers nothing without a PR, or when there is nothing to post", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      await togglePalette(user);
      expect(screen.queryByRole("option", { name: /self-reviewed/ })).toBeNull();
      await togglePalette(user);
      // Someone else's PR.
      act(() => store.getState().setFocused("dashpay/dash-evo-tool#1213"));
      await togglePalette(user);
      expect(screen.queryByRole("option", { name: /self-reviewed/ })).toBeNull();
    });
  });

  describe("tracked contributors", () => {
    it("offers each tracked contributor, mine, and tracking someone new", async () => {
      const user = userEvent.setup();
      const { store } = await setup(["OrbitWren", "tidepools"]);
      await togglePalette(user);
      for (const name of [
        "Show mine",
        "Show OrbitWren's pull requests",
        "Show tidepools' pull requests",
        "Track contributor…",
      ]) {
        expect(screen.getByRole("option", { name: new RegExp(`^${name}`) })).toBeInTheDocument();
      }
      await user.keyboard("orbitwren's");
      expect(screen.getAllByRole("option")[0]).toHaveTextContent("Show OrbitWren's pull requests");
      await user.keyboard("{Enter}");
      expect(store.getState().view).toEqual({ kind: "contributor", login: "OrbitWren" });
      expect(store.getState().expandedId).toBeNull();
      expect(store.getState().paletteOpen).toBe(false);

      await togglePalette(user);
      await user.keyboard("show mine{Enter}");
      expect(store.getState().view).toEqual({ kind: "mine" });
    });

    it("opens the Track contributor dialog", async () => {
      const user = userEvent.setup();
      const { store } = await setup();
      await togglePalette(user);
      expect(screen.queryByRole("option", { name: /^Show .*'s? pull requests/ })).toBeNull();
      await user.keyboard("track contributor{Enter}");
      const dialog = screen.getByRole("dialog", { name: "Track contributor" });
      await user.type(within(dialog).getByRole("textbox", { name: "GitHub username" }), "kestrel-dev{Enter}");
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(store.getState().view).toEqual({ kind: "contributor", login: "kestrel-dev" });
    });

    it("jumps to a tracked contributor's PR in their view", async () => {
      const user = userEvent.setup();
      const { store } = await setup(["LumenAsh"]);
      await togglePalette(user);
      await user.keyboard("1229{Enter}");
      expect(store.getState().view).toEqual({ kind: "contributor", login: "LumenAsh" });
      expect(store.getState().expandedId).toBe("dashpay/dash-evo-tool#1229");
      expect(document.querySelector('[data-pr-row="dashpay/dash-evo-tool#1229"]')).not.toBeNull();
    });
  });
});
