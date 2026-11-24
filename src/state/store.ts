// App state. One store holds the latest snapshot, the config and the UI
// choices (view, grouping, sorting, filter text, which row is open).
//
// Components read it with `useApp(selector)`. Tests build their own store
// with `createAppStore(createMockApi({ latencyMs: 0 }))` and wrap the
// component in `<StoreProvider store={...}>`.

import { createContext, createElement, useContext, type ReactNode } from "react";
import { createStore, useStore, type StoreApi } from "zustand";
import type { Api, Unsubscribe } from "../api";
import type {
  Config,
  KnownContributor,
  MergeCheck,
  PrSummary,
  RunJob,
  RunOptions,
  RunPlatform,
  RunProject,
  RunRequest,
  RunSourceKind,
  SelfReviewCheck,
  Snapshot,
} from "../types";
import { sameLogin, type View } from "../lib/select";

export type GroupBy = "project" | "status";
export type SortBy = "finishesFirst" | "updated";

/** How many log lines of a run the UI keeps. The complete log is in the job's file. */
export const RUN_LOG_LIMIT = 2000;

/** What the user last chose in the run dialog, offered again next time. */
export interface RunPrefs {
  /** null until the user has run something: use the first source offered. */
  sourceKind: RunSourceKind | null;
  /** Platform -> `RunDevice.id`. */
  devices: Partial<Record<RunPlatform, string>>;
}

export interface AppState {
  /** null until the first snapshot arrives. */
  snapshot: Snapshot | null;
  config: Config | null;
  /** True from start-up until the first load settles. */
  loading: boolean;
  /** Set when the first load failed outright. */
  loadError: string | null;
  syncing: boolean;

  view: View;
  groupBy: GroupBy;
  sortBy: SortBy;
  /**
   * Whether project groups are split into one sub-group per subcrate (the
   * area in the PR title). Has no effect while grouping by status. Off by
   * default. Remembered between runs.
   */
  bySubcrate: boolean;
  /** Filter text typed into the quick filter. */
  query: string;
  /**
   * Milestones the user has switched off, by title. `NO_MILESTONE` stands for
   * PRs without one. Everything not listed is shown, so a milestone that
   * appears later is on by default. Remembered between runs.
   */
  hiddenMilestones: string[];
  /**
   * Areas the user has switched off, per project: repo (`owner/name`) -> area
   * keys. An area is the scope in a PR title, e.g. "sdk" in "fix(sdk): ...";
   * `NO_AREA` stands for titles without one. Everything not listed is shown.
   * Remembered between runs.
   */
  hiddenAreas: Record<string, string[]>;
  /** Whether draft pull requests are listed. On by default. Remembered between runs. */
  showDrafts: boolean;
  /**
   * Whether only pull requests created in the last 3 days are listed. Off by
   * default. Remembered between runs.
   */
  recentOnly: boolean;
  /** The PR whose detail row is open, if any. */
  expandedId: string | null;
  /** The PR the keyboard cursor is on, if any. */
  focusedId: string | null;
  paletteOpen: boolean;

  /**
   * What can be run for each tracked project (see `lib`/components under
   * `run`). Empty until loaded, and when nothing can be run.
   */
  runTargets: RunProject[];
  /** The running example-app job or the last one. null when none has run since launch. */
  runJob: RunJob | null;
  /** The last lines of `runJob`'s log, oldest first, at most `RUN_LOG_LIMIT`. */
  runLog: string[];
  /** Whether the run progress panel is showing. Opens by itself when a job starts. */
  runPanelOpen: boolean;
  /** Remembered between runs. */
  runPrefs: RunPrefs;

  /** Loads the first snapshot and config, then follows backend events. */
  init(): Promise<Unsubscribe>;
  refresh(): Promise<void>;
  setView(view: View): void;
  setGroupBy(groupBy: GroupBy): void;
  setSortBy(sortBy: SortBy): void;
  setBySubcrate(on: boolean): void;
  setQuery(query: string): void;
  /** Shows or hides one milestone (a title, or `NO_MILESTONE`). */
  toggleMilestone(key: string): void;
  /** Shows every milestone again. */
  showAllMilestones(): void;
  /** Shows or hides one area (a scope, or `NO_AREA`) of one project. */
  toggleArea(repo: string, key: string): void;
  /** Shows every area of one project again. */
  showAllAreas(repo: string): void;
  /** Lists or hides draft pull requests. */
  setShowDrafts(show: boolean): void;
  /** Lists only recently created pull requests, or all of them again. */
  setRecentOnly(on: boolean): void;
  toggleExpanded(id: string): void;
  setFocused(id: string | null): void;
  setPaletteOpen(open: boolean): void;
  /** These four reject with a message fit to show the user. */
  addProject(input: string): Promise<void>;
  removeProject(repo: string): Promise<void>;
  followPr(input: string): Promise<void>;
  unfollowPr(id: string): Promise<void>;
  /**
   * Starts tracking a contributor (a login, `@login` or a profile URL) and
   * switches to their view. Rejects with a message fit to show the user.
   */
  trackContributor(input: string): Promise<void>;
  /** Stops tracking; leaves their view if it is showing. Rejects with a message fit to show the user. */
  untrackContributor(login: string): Promise<void>;
  /** People the app can offer to track. Not cached. Rejects with a readable message. */
  knownContributors(): Promise<KnownContributor[]>;
  /** Every open PR in a tracked project, for the follow picker. Not cached. */
  listOpenPrs(repo: string): Promise<PrSummary[]>;
  /** A fresh merge check from GitHub, for the merge confirmation. Rejects with a readable message. */
  mergeCheck(id: string): Promise<MergeCheck>;
  /** Squash-merges after the user has confirmed. Rejects with a readable message. */
  squashMerge(id: string, expectedHeadSha: string): Promise<void>;
  /** A fresh self-review check from GitHub, for the confirmation. Rejects with a readable message. */
  selfReviewCheck(id: string): Promise<SelfReviewCheck>;
  /** Posts `/self-reviewed <expectedHeadSha>` after the user has confirmed. Rejects with a readable message. */
  postSelfReview(id: string, expectedHeadSha: string): Promise<void>;
  openUrl(url: string): Promise<void>;

  /** Re-reads `runTargets`. Never rejects; on failure the old value stays. */
  loadRunTargets(): Promise<void>;
  /** Sets (or with null, forgets) where a project's local clone is. Rejects with a readable message. */
  setClone(repo: string, path: string | null): Promise<void>;
  /** The system folder picker. null when the user cancels. */
  pickFolder(): Promise<string | null>;
  /** Sources and devices for running a PR. Not cached. Rejects with a readable message. */
  runOptions(id: string): Promise<RunOptions>;
  /** Starts a run, remembers the choices and opens the progress panel. Rejects with a readable message. */
  runStart(request: RunRequest): Promise<void>;
  /** Stops the running job. Rejects with a readable message. */
  runCancel(): Promise<void>;
  /** Opens the complete log file. Rejects with a readable message. */
  runOpenLog(): Promise<void>;
  setRunPanelOpen(open: boolean): void;
}

export type AppStore = StoreApi<AppState>;

const PREFS_KEY = "pr-status.prefs.v1";

/** Key for "PRs with no milestone" in `hiddenMilestones`. No real title is empty. */
export const NO_MILESTONE = "";

interface Prefs {
  view: View;
  groupBy: GroupBy;
  sortBy: SortBy;
  bySubcrate: boolean;
  hiddenMilestones: string[];
  hiddenAreas: Record<string, string[]>;
  showDrafts: boolean;
  recentOnly: boolean;
  runPrefs: RunPrefs;
}

/** Keeps what is well-formed in saved run choices; anything else is dropped. */
function cleanRunPrefs(raw: unknown): RunPrefs {
  const out: RunPrefs = { sourceKind: null, devices: {} };
  if (!raw || typeof raw !== "object") return out;
  const r = raw as { sourceKind?: unknown; devices?: unknown };
  if (r.sourceKind === "prHead" || r.sourceKind === "worktree") out.sourceKind = r.sourceKind;
  if (r.devices && typeof r.devices === "object") {
    const d = r.devices as Record<string, unknown>;
    if (typeof d.ios === "string") out.devices.ios = d.ios;
    if (typeof d.android === "string") out.devices.android = d.android;
  }
  return out;
}

/** Key for "PRs whose title has no scope" in `hiddenAreas`. No real scope is empty. */
export const NO_AREA = "";

/** Keeps only `repo -> string[]` entries; anything else in saved prefs is dropped. */
function cleanHiddenAreas(raw: unknown): Record<string, string[]> {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  const out: Record<string, string[]> = {};
  for (const [repo, keys] of Object.entries(raw as Record<string, unknown>)) {
    if (!Array.isArray(keys)) continue;
    const clean = keys.filter((k): k is string => typeof k === "string");
    if (clean.length) out[repo] = clean;
  }
  return out;
}

function loadPrefs(): Partial<Prefs> {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as Partial<Prefs>;
  } catch {
    return {};
  }
}

function savePrefs(state: AppState): void {
  try {
    const prefs: Prefs = { view: state.view, groupBy: state.groupBy, sortBy: state.sortBy,
      bySubcrate: state.bySubcrate, hiddenMilestones: state.hiddenMilestones, hiddenAreas: state.hiddenAreas,
      showDrafts: state.showDrafts, recentOnly: state.recentOnly, runPrefs: state.runPrefs };
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Storage can be unavailable; the choice then lasts for this session only.
  }
}

/** Backend errors arrive as strings; anything else gets a plain fallback. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong.";
}

export function createAppStore(api: Api): AppStore {
  const prefs = loadPrefs();
  return createStore<AppState>()((set, get) => {
    const setAndSave = (patch: Partial<AppState>) => {
      set(patch);
      savePrefs(get());
    };
    return {
      snapshot: null,
      config: null,
      loading: true,
      loadError: null,
      syncing: false,
      view: prefs.view ?? { kind: "mine" },
      groupBy: prefs.groupBy ?? "project",
      sortBy: prefs.sortBy ?? "finishesFirst",
      bySubcrate: prefs.bySubcrate === true,
      query: "",
      hiddenMilestones: Array.isArray(prefs.hiddenMilestones)
        ? prefs.hiddenMilestones.filter((m) => typeof m === "string")
        : [],
      hiddenAreas: cleanHiddenAreas(prefs.hiddenAreas),
      showDrafts: prefs.showDrafts !== false,
      recentOnly: prefs.recentOnly === true,
      expandedId: null,
      focusedId: null,
      paletteOpen: false,
      runTargets: [],
      runJob: null,
      runLog: [],
      runPanelOpen: false,
      runPrefs: cleanRunPrefs(prefs.runPrefs),

      async init() {
        const unsubs = await Promise.all([
          api.onSnapshot((snapshot) => set({ snapshot, loading: false, loadError: null })),
          api.onSyncState(({ syncing }) => set({ syncing })),
          api.onRunJob((runJob) =>
            // A new job starts with an empty log.
            set(get().runJob?.jobId === runJob.jobId ? { runJob } : { runJob, runLog: [] }),
          ),
          api.onRunLog(({ jobId, lines }) => {
            if (get().runJob?.jobId !== jobId) return;
            set({ runLog: [...get().runLog, ...lines].slice(-RUN_LOG_LIMIT) });
          }),
        ]);
        // Running example apps is an extra: the list must load without it.
        void get().loadRunTargets();
        void Promise.all([api.runJob(), api.runLog()])
          .then(([runJob, runLog]) => {
            // Events that arrived while we asked are newer than the answer.
            if (runJob && !get().runJob) set({ runJob, runLog: runLog.slice(-RUN_LOG_LIMIT) });
          })
          .catch(() => undefined);
        try {
          const [snapshot, config] = await Promise.all([api.getSnapshot(), api.getConfig()]);
          // A null snapshot means the first sync is still running; stay in
          // `loading` until the snapshot event arrives.
          set(snapshot ? { snapshot, config, loading: false, loadError: null } : { config, loadError: null });
          const view = get().view;
          if (view.kind === "project" && !config.projects.includes(view.repo)) {
            set({ view: { kind: "mine" } });
          }
          if (view.kind === "contributor" && !config.contributors.some((c) => sameLogin(c, view.login))) {
            set({ view: { kind: "mine" } });
          }
        } catch (e) {
          set({ loading: false, loadError: errorMessage(e) });
        }
        return () => unsubs.forEach((u) => u());
      },

      async refresh() {
        try {
          await api.refresh();
        } catch (e) {
          set({ loadError: errorMessage(e) });
        }
      },

      setView: (view) => setAndSave({ view, expandedId: null, focusedId: null }),
      setGroupBy: (groupBy) => setAndSave({ groupBy }),
      setSortBy: (sortBy) => setAndSave({ sortBy }),
      setBySubcrate: (bySubcrate) => setAndSave({ bySubcrate }),
      setQuery: (query) => set({ query }),
      toggleMilestone: (key) => {
        const hidden = get().hiddenMilestones;
        setAndSave({ hiddenMilestones: hidden.includes(key) ? hidden.filter((m) => m !== key) : [...hidden, key] });
      },
      showAllMilestones: () => setAndSave({ hiddenMilestones: [] }),
      toggleArea: (repo, key) => {
        const all = get().hiddenAreas;
        const hidden = all[repo] ?? [];
        const next = hidden.includes(key) ? hidden.filter((k) => k !== key) : [...hidden, key];
        const rest = Object.fromEntries(Object.entries(all).filter(([r]) => r !== repo));
        setAndSave({ hiddenAreas: next.length ? { ...rest, [repo]: next } : rest });
      },
      setShowDrafts: (showDrafts) => setAndSave({ showDrafts }),
      setRecentOnly: (recentOnly) => setAndSave({ recentOnly }),
      showAllAreas: (repo) => {
        setAndSave({ hiddenAreas: Object.fromEntries(Object.entries(get().hiddenAreas).filter(([r]) => r !== repo)) });
      },
      toggleExpanded: (id) => set({ expandedId: get().expandedId === id ? null : id, focusedId: id }),
      setFocused: (focusedId) => set({ focusedId }),
      setPaletteOpen: (paletteOpen) => set({ paletteOpen }),

      async addProject(input) {
        set({ config: await api.addProject(input) });
        void get().loadRunTargets();
      },
      async removeProject(repo) {
        const config = await api.removeProject(repo);
        const view = get().view;
        set({ config });
        if (view.kind === "project" && view.repo === repo) setAndSave({ view: { kind: "mine" } });
        void get().loadRunTargets();
      },
      async followPr(input) {
        set({ config: await api.followPr(input) });
      },
      async unfollowPr(id) {
        set({ config: await api.unfollowPr(id) });
      },
      async trackContributor(input) {
        const before = get().config?.contributors ?? [];
        const config = await api.trackContributor(input);
        set({ config });
        // The backend stores GitHub's casing; find who was added, or who was meant if already tracked.
        const typed = input.trim().replace(/^.*\//, "").replace(/^@/, "");
        const login =
          config.contributors.find((c) => !before.some((b) => sameLogin(b, c))) ??
          config.contributors.find((c) => sameLogin(c, typed));
        if (login) setAndSave({ view: { kind: "contributor", login }, expandedId: null, focusedId: null });
      },
      async untrackContributor(login) {
        const config = await api.untrackContributor(login);
        const view = get().view;
        set({ config });
        if (view.kind === "contributor" && sameLogin(view.login, login)) setAndSave({ view: { kind: "mine" } });
      },
      knownContributors: () => api.knownContributors(),
      listOpenPrs: (repo) => api.listOpenPrs(repo),
      mergeCheck: (id) => api.mergeCheck(id),
      squashMerge: (id, expectedHeadSha) => api.squashMerge(id, expectedHeadSha),
      selfReviewCheck: (id) => api.selfReviewCheck(id),
      postSelfReview: (id, expectedHeadSha) => api.postSelfReview(id, expectedHeadSha),
      openUrl: (url) => api.openUrl(url),

      async loadRunTargets() {
        try {
          set({ runTargets: await api.runTargets() });
        } catch {
          // Keep what we had; nothing to run is the safe reading of a failure at start-up.
        }
      },
      async setClone(repo, path) {
        set({ runTargets: await api.setClone(repo, path) });
      },
      pickFolder: () => api.pickFolder(),
      runOptions: (id) => api.runOptions(id),
      async runStart(request) {
        const runJob = await api.runStart(request);
        // The job's first event may have arrived before the call returned.
        const known = get().runJob?.jobId === runJob.jobId;
        set(known ? { runPanelOpen: true } : { runJob, runLog: [], runPanelOpen: true });
        const prefs = get().runPrefs;
        setAndSave({
          runPrefs: {
            sourceKind: request.sourceKind,
            devices: { ...prefs.devices, [request.platform]: request.deviceId },
          },
        });
      },
      runCancel: () => api.runCancel(),
      runOpenLog: () => api.runOpenLog(),
      setRunPanelOpen: (runPanelOpen) => set({ runPanelOpen }),
    };
  });
}

const StoreContext = createContext<AppStore | null>(null);

export function StoreProvider({ store, children }: { store: AppStore; children: ReactNode }) {
  return createElement(StoreContext.Provider, { value: store }, children);
}

export function useApp<T>(selector: (state: AppState) => T): T {
  const store = useContext(StoreContext);
  if (!store) throw new Error("useApp must be used inside <StoreProvider>");
  return useStore(store, selector);
}
