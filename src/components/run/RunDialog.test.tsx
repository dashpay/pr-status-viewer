import { describe, expect, it } from "vitest";
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RunOptions } from "../../types";
import { FIRST_BUILD_NOTE, T3_CAUTION } from "./RunDialog";
import { PR_6033, PR_6034, PR_6038, detail, flush, renderApp, row, runDialog, savePrefs } from "./testUtils";

type User = ReturnType<typeof userEvent.setup>;

async function openDialog(user: User, id: string, n: number, platform = "iOS") {
  await user.click(row(id));
  await user.click(within(detail(n)).getByRole("button", { name: `Run on ${platform}` }));
  await flush();
  return runDialog();
}

const radio = (name: RegExp) => within(runDialog()).getByRole("radio", { name });
const deviceSelect = () => within(runDialog()).getByRole("combobox", { name: "Device" }) as HTMLSelectElement;
const buildButton = () => within(runDialog()).getByRole("button", { name: "Build and run" });

describe("run buttons in the detail", () => {
  it("offers both apps for a project that has them", async () => {
    const user = userEvent.setup();
    await renderApp();
    await user.click(row(PR_6033));
    const actions = within(detail(6033));
    expect(actions.getByRole("button", { name: "Run on iOS" })).toBeInTheDocument();
    expect(actions.getByRole("button", { name: "Run on Android" })).toBeInTheDocument();
  });

  it("offers only the platforms the project has", async () => {
    const user = userEvent.setup();
    await renderApp(
      {},
      {
        runTargets: async () => [{ repo: "dashpay/platform", clonePath: "/x", cloneIsSet: false, platforms: ["android"] }],
      },
    );
    await user.click(row(PR_6033));
    expect(within(detail(6033)).queryByRole("button", { name: "Run on iOS" })).toBeNull();
    expect(within(detail(6033)).getByRole("button", { name: "Run on Android" })).toBeInTheDocument();
  });

  it("shows nothing for a project without example apps", async () => {
    const user = userEvent.setup();
    const { store } = await renderApp();
    act(() => store.getState().setView({ kind: "project", repo: "dashpay/dash-evo-tool" }));
    await user.click(row("dashpay/dash-evo-tool#1213"));
    expect(within(detail(1213)).queryByRole("button", { name: /^Run on/ })).toBeNull();
  });
});

describe("the run dialog", () => {
  it("asks for options, starts on the platform clicked and picks the first source and device", async () => {
    const user = userEvent.setup();
    await renderApp();
    const dialog = await openDialog(user, PR_6033, 6033);
    expect(within(dialog).getByRole("heading", { name: "Run #6033" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "iOS" })).toHaveAttribute("aria-pressed", "true");
    expect(radio(/Pull request head · 5cc1bd0/)).toBeChecked();
    expect(within(dialog).getByText("A clean copy that PR Status keeps for itself")).toBeInTheDocument();
    expect(radio(/Your worktree · fix\/swift-sdk-dpns-contests/)).not.toBeChecked();
    expect(within(dialog).getByText("At the pull request head")).toBeInTheDocument();
    expect(within(dialog).getByText("~/src/platform-worktrees/fix-swift-sdk-dpns-contests")).toBeInTheDocument();
    expect(deviceSelect().selectedOptions[0]).toHaveTextContent("iPhone 17 Pro · iOS 26.1 · Running");
    expect(within(dialog).getByText(FIRST_BUILD_NOTE)).toBeInTheDocument();
    // Focus moves to the primary button once the choices are in.
    expect(buildButton()).toHaveFocus();
  });

  it("starts on Android when that was clicked, and switches platform inside", async () => {
    const user = userEvent.setup();
    await renderApp();
    const dialog = await openDialog(user, PR_6033, 6033, "Android");
    expect(within(dialog).getByRole("button", { name: "Android" })).toHaveAttribute("aria-pressed", "true");
    expect(deviceSelect().selectedOptions[0]).toHaveTextContent("palmatum_api35");
    await user.click(within(dialog).getByRole("button", { name: "iOS" }));
    expect(deviceSelect().selectedOptions[0]).toHaveTextContent("iPhone 17 Pro");
  });

  it("uses the remembered source kind and device", async () => {
    savePrefs({ sourceKind: "worktree", devices: { ios: "0B6F2C9E-17" } });
    const user = userEvent.setup();
    await renderApp();
    await openDialog(user, PR_6033, 6033);
    expect(radio(/Your worktree/)).toBeChecked();
    expect(deviceSelect().selectedOptions[0]).toHaveTextContent(/^iPhone 17 · iOS 26.1$/);
  });

  it("falls back to the first source and device when the remembered ones are not offered", async () => {
    savePrefs({ sourceKind: "worktree", devices: { ios: "no-such-simulator" } });
    const user = userEvent.setup();
    await renderApp();
    await openDialog(user, PR_6038, 6038);
    expect(within(runDialog()).getAllByRole("radio")).toHaveLength(1);
    expect(radio(/Pull request head/)).toBeChecked();
    expect(deviceSelect().selectedOptions[0]).toHaveTextContent("iPhone 17 Pro");
  });

  it("describes a dirty worktree off the head, and cautions while T3 Code works on the PR", async () => {
    const user = userEvent.setup();
    await renderApp();
    const dialog = await openDialog(user, PR_6034, 6034);
    expect(within(dialog).getByText(/Not at the pull request head:/)).toHaveTextContent("def5678");
    expect(within(dialog).getByText(/Has uncommitted changes/)).toBeInTheDocument();
    expect(within(dialog).queryByText(T3_CAUTION)).toBeNull();
    await user.click(radio(/Your worktree/));
    expect(within(dialog).getByText(T3_CAUTION)).toBeInTheDocument();
  });

  it("shows why a platform cannot run and disables the primary button", async () => {
    const user = userEvent.setup();
    await renderApp({ runUnavailable: { ios: "Xcode is not installed." } });
    const dialog = await openDialog(user, PR_6033, 6033);
    expect(within(dialog).getByText("Xcode is not installed.")).toBeInTheDocument();
    expect(within(dialog).queryByRole("combobox")).toBeNull();
    expect(buildButton()).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "Android" }));
    expect(buildButton()).toBeEnabled();
  });

  it("shows a quiet loading state, then the rejection message", async () => {
    let reject!: (e: unknown) => void;
    const pending = new Promise<RunOptions>((_, rej) => (reject = rej));
    const user = userEvent.setup();
    await renderApp({}, { runOptions: () => pending });
    await user.click(row(PR_6033));
    await user.click(within(detail(6033)).getByRole("button", { name: "Run on iOS" }));
    expect(within(runDialog()).getByRole("status")).toHaveTextContent("Looking for worktrees and devices");
    expect(buildButton()).toBeDisabled();
    await act(async () => reject("Git is not installed."));
    expect(within(runDialog()).getByRole("alert")).toHaveTextContent("Git is not installed.");
  });

  it("starts the run, remembers the choices, closes and opens the progress panel", async () => {
    const user = userEvent.setup();
    const { store } = await renderApp({ runStepMs: 60_000 });
    await openDialog(user, PR_6033, 6033);
    await user.selectOptions(deviceSelect(), "iPhone Air · iOS 26.1");
    await user.click(radio(/Your worktree/));
    await user.click(buildButton());
    await flush();
    expect(screen.queryByRole("dialog")).toBeNull();
    const state = store.getState();
    expect(state.runPanelOpen).toBe(true);
    expect(state.runJob).toMatchObject({ prId: PR_6033, platform: "ios", deviceName: "iPhone Air", sourceKind: "worktree" });
    expect(state.runPrefs).toEqual({ sourceKind: "worktree", devices: { ios: "8E3A41D2-AIR" } });
    expect(screen.getByRole("region", { name: "iOS · #6033 · iPhone Air" })).toBeInTheDocument();
  });

  it("returns focus to the button that opened it", async () => {
    const user = userEvent.setup();
    await renderApp();
    await openDialog(user, PR_6033, 6033);
    await user.click(within(runDialog()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(within(detail(6033)).getByRole("button", { name: "Run on iOS" })).toHaveFocus();
  });

  it("says another run is in progress and offers to show it", async () => {
    const user = userEvent.setup();
    const { store } = await renderApp({ runStepMs: 60_000 });
    await openDialog(user, PR_6033, 6033);
    await user.click(buildButton());
    await flush();
    act(() => store.getState().setRunPanelOpen(false));

    const dialog = await openDialog(user, PR_6034, 6034, "Android");
    expect(within(dialog).getByText("Another run is in progress")).toBeInTheDocument();
    expect(buildButton()).toBeDisabled();
    expect(buildButton()).toHaveAccessibleDescription("Another run is in progress");
    await user.click(within(dialog).getByRole("button", { name: "Show progress" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(store.getState().runPanelOpen).toBe(true);
  });

  it("shows the message when the backend refuses to start", async () => {
    const user = userEvent.setup();
    await renderApp({}, { runStart: async () => Promise.reject("The simulator runtime is missing.") });
    await openDialog(user, PR_6033, 6033);
    await user.click(buildButton());
    await flush();
    expect(within(runDialog()).getByRole("alert")).toHaveTextContent("The simulator runtime is missing.");
    expect(buildButton()).toBeEnabled();
  });
});
