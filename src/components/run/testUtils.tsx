// Test helpers for the run UI: the whole app (shell, list, palette) around a
// mock-backed store, and a few lookups.
import { act, render, screen, within } from "@testing-library/react";
import type { Api } from "../../api";
import { createMockApi, type MockApiOptions } from "../../mock/mockApi";
import { createAppStore, StoreProvider, type AppStore, type RunPrefs } from "../../state/store";
import { Shell } from "../shell/Shell";
import { PrTable } from "../table/PrTable";
import { CommandPalette } from "../palette/CommandPalette";

export const PR_6033 = "dashpay/platform#6033";
export const PR_6034 = "dashpay/platform#6034";
export const PR_6038 = "dashpay/platform#6038";

export async function renderApp(
  options: MockApiOptions = {},
  overrides: Partial<Api> = {},
): Promise<{ store: AppStore; api: Api }> {
  const api: Api = { ...createMockApi({ latencyMs: 0, ...options }), ...overrides };
  const store = createAppStore(api);
  render(
    <StoreProvider store={store}>
      <Shell>
        <PrTable />
      </Shell>
      <CommandPalette />
    </StoreProvider>,
  );
  await act(async () => {
    await store.getState().init();
    await store.getState().loadRunTargets();
  });
  return { store, api };
}

/** Saved run choices, as the store reads them at start-up. Call before `renderApp`. */
export function savePrefs(runPrefs: RunPrefs): void {
  localStorage.setItem("pr-status.prefs.v1", JSON.stringify({ runPrefs }));
}

export const row = (id: string) => document.querySelector<HTMLElement>(`[data-pr-row="${id}"]`) as HTMLElement;

export const detail = (n: number) => screen.getByRole("region", { name: `Details for #${n}` });

/** Lets resolved promises and zero-delay timers run, inside act. Works with real and fake timers. */
export async function flush(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });
}

export const runDialog = () => screen.getByRole("dialog", { name: /^Run #/ });
export const panel = () => screen.getByRole("region", { name: /^(iOS|Android) · #/ });
export const steps = () => within(panel()).getByRole("list", { name: "Steps" });
export const stepItem = (label: string) =>
  within(steps())
    .getAllByRole("listitem")
    .find((li) => li.textContent?.includes(label)) as HTMLElement;
export const headerStatus = () => screen.getByRole("button", { name: /^(Running on|Opened on|iOS run|Android run)/ });
