// Test helpers: render the shell around a mock-backed store.
import { act, render } from "@testing-library/react";
import type { ReactNode } from "react";
import type { Api } from "../../api";
import { createMockApi, type MockApiOptions } from "../../mock/mockApi";
import { createAppStore, StoreProvider, type AppStore } from "../../state/store";
import { Shell } from "./Shell";

export function makeApi(options: MockApiOptions = {}, overrides: Partial<Api> = {}): Api {
  return { ...createMockApi({ latencyMs: 0, ...options }), ...overrides };
}

export async function renderShell(
  api: Api = makeApi(),
  { init = true, children = <div data-testid="table">table</div> }: { init?: boolean; children?: ReactNode } = {},
): Promise<{ store: AppStore; api: Api }> {
  const store = createAppStore(api);
  render(
    <StoreProvider store={store}>
      <Shell>{children}</Shell>
    </StoreProvider>,
  );
  if (init) {
    await act(async () => {
      await store.getState().init();
    });
  }
  return { store, api };
}
