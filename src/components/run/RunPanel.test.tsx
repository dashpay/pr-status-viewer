import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, within } from "@testing-library/react";
import type { Api } from "../../api";
import type { MockApiOptions } from "../../mock/mockApi";
import type { AppStore } from "../../state/store";
import { PR_6033, detail, flush, headerStatus, panel, renderApp, row, runDialog, stepItem } from "./testUtils";

const STEP_MS = 4000;

/** Advances the fake clock and lets the store and React catch up. */
async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** Opens #6033's detail, the iOS dialog, and starts the run with its defaults. */
async function startRun(options: MockApiOptions = {}, overrides: Partial<Api> = {}): Promise<AppStore> {
  const { store } = await renderApp({ runStepMs: STEP_MS, ...options }, overrides);
  fireEvent.click(row(PR_6033));
  fireEvent.click(within(detail(6033)).getByRole("button", { name: "Run on iOS" }));
  await flush();
  fireEvent.click(within(runDialog()).getByRole("button", { name: "Build and run" }));
  await flush();
  return store;
}

const marker = () => within(row(PR_6033)).queryByText(/^Running on/);
const live = () => document.querySelector(".run-status__live") as HTMLElement;

beforeEach(() => {
  vi.useFakeTimers({ now: Date.parse("2026-11-22T10:00:00Z") });
});
afterEach(() => {
  vi.useRealTimers();
});

describe("the run panel", () => {
  it("follows a run from start to an opened app", async () => {
    await startRun();
    expect(screen.queryByRole("dialog")).toBeNull();
    const p = panel();
    expect(within(p).getByRole("heading", { name: "iOS · #6033 · iPhone 17 Pro" })).toBeInTheDocument();
    expect(p).toHaveTextContent("Running · 0 s");
    expect(p).toHaveTextContent("Pull request head · 5cc1bd0");
    expect(stepItem("Check out the code")).toHaveTextContent("Running");
    expect(stepItem("Check out the code")).toHaveAttribute("aria-current", "step");
    expect(stepItem("Build the SDK")).toHaveTextContent("Not started");
    expect(headerStatus()).toHaveTextContent("Running on iOS · Check out the code");
    expect(headerStatus()).toHaveAttribute("aria-expanded", "true");
    expect(live()).toHaveTextContent("Running on iOS · Check out the code");
    expect(marker()).toHaveTextContent("Running on iOS");
    expect(within(p).getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(within(p).getByRole("button", { name: "Open full log" })).toBeInTheDocument();

    await advance(STEP_MS);
    expect(stepItem("Check out the code")).toHaveTextContent("Done · 4 s");
    expect(stepItem("Build the SDK")).toHaveTextContent("Running");
    expect(headerStatus()).toHaveTextContent("Running on iOS · Build the SDK");
    expect(panel()).toHaveTextContent("Running · 4 s");
    const log = within(panel()).getByRole("region", { name: "Log" });
    expect(log).toHaveTextContent("$ git fetch origin 5cc1bd0");
    expect(log).toHaveAttribute("aria-live", "off");

    await advance(STEP_MS * 5);
    expect(panel()).toHaveTextContent("Opened · 24 s");
    for (const label of ["Build the app", "Start the simulator", "Install the app", "Open the app"]) {
      expect(stepItem(label)).toHaveTextContent("Done · 4 s");
    }
    expect(within(panel()).queryByRole("button", { name: "Cancel" })).toBeNull();
    expect(headerStatus()).toHaveTextContent("Opened on iPhone 17 Pro");
    expect(live()).toHaveTextContent("Opened on iPhone 17 Pro");
    expect(marker()).toBeNull();
  });

  it("shows the error when a run fails", async () => {
    await startRun({ runFailStep: "buildApp" });
    await advance(STEP_MS * 3);
    const p = panel();
    expect(p).toHaveTextContent("Failed · 12 s");
    expect(p).toHaveTextContent("The app did not build: xcodebuild exited with status 65.");
    expect(stepItem("Build the app")).toHaveTextContent("Failed · 4 s");
    expect(stepItem("Start the simulator")).toHaveTextContent("Not started");
    expect(headerStatus()).toHaveTextContent("iOS run failed");
    expect(live()).toHaveTextContent("iOS run failed: The app did not build");
    expect(marker()).toBeNull();
  });

  it("cancels a run", async () => {
    const store = await startRun();
    await advance(STEP_MS + 100);
    fireEvent.click(within(panel()).getByRole("button", { name: "Cancel" }));
    await flush();
    expect(store.getState().runJob?.state).toBe("cancelled");
    expect(panel()).toHaveTextContent("Cancelled");
    expect(stepItem("Check out the code")).toHaveTextContent("Done");
    expect(stepItem("Build the SDK")).toHaveTextContent("Stopped");
    expect(headerStatus()).toHaveTextContent("iOS run cancelled");
    expect(marker()).toBeNull();
    // Nothing moves after a cancel.
    await advance(STEP_MS * 10);
    expect(store.getState().runJob?.state).toBe("cancelled");
  });

  it("hides and comes back from the header, keeping the list usable", async () => {
    await startRun();
    fireEvent.click(within(panel()).getByRole("button", { name: "Hide" }));
    expect(screen.queryByRole("region", { name: /^iOS · #/ })).toBeNull();
    expect(headerStatus()).toHaveFocus();
    expect(headerStatus()).toHaveAttribute("aria-expanded", "false");
    // The run goes on while hidden.
    await advance(STEP_MS);
    expect(headerStatus()).toHaveTextContent("Build the SDK");
    fireEvent.click(headerStatus());
    expect(panel()).toBeInTheDocument();
    // Esc inside the panel hides it too.
    fireEvent.keyDown(within(panel()).getByRole("button", { name: "Hide" }), { key: "Escape" });
    expect(screen.queryByRole("region", { name: /^iOS · #/ })).toBeNull();
    // The list is still there and its rows still open.
    expect(row(PR_6033)).toHaveAttribute("aria-expanded", "true");
  });

  it("opens the full log, and shows why when it cannot", async () => {
    const runOpenLog = vi.fn(async () => undefined);
    await startRun({}, { runOpenLog });
    const button = within(panel()).getByRole("button", { name: "Open full log" });
    expect(button).toHaveAttribute("title", expect.stringContaining("run-1.log"));
    fireEvent.click(button);
    await flush();
    expect(runOpenLog).toHaveBeenCalledOnce();
    expect(within(panel()).queryByRole("alert")).toBeNull();
    runOpenLog.mockRejectedValueOnce("The log file is gone.");
    fireEvent.click(button);
    await flush();
    expect(within(panel()).getByRole("alert")).toHaveTextContent("The log file is gone.");
  });

  it("follows the log unless the user scrolls up, then offers to jump", async () => {
    await startRun({ runLinesPerBatch: 50 });
    const log = within(panel()).getByRole("region", { name: "Log" });
    // jsdom has no layout: give the log a size.
    let scrollHeight = 2000;
    Object.defineProperty(log, "scrollHeight", { configurable: true, get: () => scrollHeight });
    Object.defineProperty(log, "clientHeight", { configurable: true, get: () => 200 });

    await advance(STEP_MS / 4);
    expect(log.scrollTop).toBe(2000);
    expect(within(panel()).queryByRole("button", { name: "Jump to latest" })).toBeNull();

    log.scrollTop = 500;
    fireEvent.scroll(log);
    scrollHeight = 3000;
    await advance(STEP_MS / 4);
    expect(log.scrollTop).toBe(500);
    const jump = within(panel()).getByRole("button", { name: "Jump to latest" });
    fireEvent.click(jump);
    expect(log.scrollTop).toBe(3000);
    expect(within(panel()).queryByRole("button", { name: "Jump to latest" })).toBeNull();
  });

  it("keeps at most 2000 log lines", async () => {
    const store = await startRun({ runLinesPerBatch: 400 });
    await advance(STEP_MS * 2);
    expect(store.getState().runLog).toHaveLength(2000);
    const text = within(panel()).getByRole("region", { name: "Log" }).textContent ?? "";
    expect(text.split("\n")).toHaveLength(2000);
  });
});
