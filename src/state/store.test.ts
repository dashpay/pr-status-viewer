import { describe, expect, it } from "vitest";
import { createMockApi } from "../mock/mockApi";
import { viewCounts } from "../lib/select";
import { createAppStore } from "./store";

describe("app store", () => {
  it("loads the snapshot and config on init", async () => {
    const store = createAppStore(createMockApi({ latencyMs: 0 }));
    expect(store.getState().loading).toBe(true);
    await store.getState().init();
    const state = store.getState();
    expect(state.loading).toBe(false);
    expect(state.config?.projects).toContain("dashpay/platform");
    expect(viewCounts(state.snapshot).mine).toBe(11);
    expect(viewCounts(state.snapshot).following).toBe(5);
  });

  it("falls back to Mine when the open project is removed", async () => {
    const store = createAppStore(createMockApi({ latencyMs: 0 }));
    await store.getState().init();
    store.getState().setView({ kind: "project", repo: "dashpay/dips" });
    await store.getState().removeProject("dashpay/dips");
    expect(store.getState().view).toEqual({ kind: "mine" });
    expect(store.getState().config?.projects).not.toContain("dashpay/dips");
  });

  it("rejects a malformed project name with a readable message", async () => {
    const store = createAppStore(createMockApi({ latencyMs: 0 }));
    await store.getState().init();
    await expect(store.getState().addProject("not a repo")).rejects.toThrow(/owner\/name/);
  });

  it("lists pull requests of any age by default, and remembers the created recently switch", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const store = createAppStore(api);
    expect(store.getState().recentOnly).toBe(false);
    store.getState().setRecentOnly(true);
    expect(store.getState().recentOnly).toBe(true);
    expect(JSON.parse(localStorage.getItem("pr-status.prefs.v1") ?? "{}").recentOnly).toBe(true);
    expect(createAppStore(api).getState().recentOnly).toBe(true);
    store.getState().setRecentOnly(false);
    expect(createAppStore(api).getState().recentOnly).toBe(false);
  });

  it("reads saved prefs from before the created recently switch, and ignores a malformed value", () => {
    const api = createMockApi({ latencyMs: 0 });
    localStorage.setItem("pr-status.prefs.v1", JSON.stringify({ groupBy: "status", showDrafts: false }));
    let state = createAppStore(api).getState();
    expect(state.recentOnly).toBe(false);
    expect(state.groupBy).toBe("status");
    expect(state.showDrafts).toBe(false);
    localStorage.setItem("pr-status.prefs.v1", JSON.stringify({ recentOnly: "yes" }));
    state = createAppStore(api).getState();
    expect(state.recentOnly).toBe(false);
  });

  it("does not split by subcrate by default, and remembers the box", () => {
    const api = createMockApi({ latencyMs: 0 });
    const store = createAppStore(api);
    expect(store.getState().bySubcrate).toBe(false);
    store.getState().setBySubcrate(true);
    expect(JSON.parse(localStorage.getItem("pr-status.prefs.v1") ?? "{}").bySubcrate).toBe(true);
    expect(createAppStore(api).getState().bySubcrate).toBe(true);
    store.getState().setBySubcrate(false);
    expect(createAppStore(api).getState().bySubcrate).toBe(false);
  });

  it("reads saved prefs from before the subcrate box, and ignores a malformed value", () => {
    const api = createMockApi({ latencyMs: 0 });
    localStorage.setItem("pr-status.prefs.v1", JSON.stringify({ groupBy: "project", sortBy: "updated" }));
    let state = createAppStore(api).getState();
    expect(state.bySubcrate).toBe(false);
    expect(state.sortBy).toBe("updated");
    localStorage.setItem("pr-status.prefs.v1", JSON.stringify({ bySubcrate: "yes" }));
    state = createAppStore(api).getState();
    expect(state.bySubcrate).toBe(false);
  });
});
