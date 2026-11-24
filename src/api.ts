// The one place the UI talks to the backend. Components and the store use
// `api`; nothing else imports from `@tauri-apps/*`.
//
// Inside the Tauri window `api` calls the Rust commands. In a plain browser
// (`npm run dev`) and in tests it is a mock backed by `mock/snapshot.ts`, so
// the UI can be built and tested without the backend.

import type {
  Config,
  KnownContributor,
  MergeCheck,
  PrSummary,
  RunJob,
  RunLog,
  RunOptions,
  RunProject,
  RunRequest,
  SelfReviewCheck,
  Snapshot,
  SyncState,
} from "./types";
import { createMockApi } from "./mock/mockApi";

export type Unsubscribe = () => void;

export interface Api {
  /** The latest snapshot the backend holds, or null before the first sync. */
  getSnapshot(): Promise<Snapshot | null>;
  getConfig(): Promise<Config>;
  /** Starts a sync now. The result arrives through `onSnapshot`. */
  refresh(): Promise<void>;
  /** `input` is `owner/name` or a GitHub URL. Rejects with a readable message. */
  addProject(input: string): Promise<Config>;
  removeProject(repo: string): Promise<Config>;
  /** `input` is a PR id (`owner/name#123`) or a pull request URL. */
  followPr(input: string): Promise<Config>;
  unfollowPr(id: string): Promise<Config>;
  /** Every open PR in a tracked project, for the follow picker. */
  listOpenPrs(repo: string): Promise<PrSummary[]>;
  /**
   * Starts tracking a contributor. `input` is a GitHub login, with or without
   * a leading `@`, or a github.com profile URL. The backend checks with GitHub
   * that the user exists and stores the login in GitHub's casing. Rejects with
   * a readable message. Tracking someone already tracked changes nothing.
   */
  trackContributor(input: string): Promise<Config>;
  untrackContributor(login: string): Promise<Config>;
  /**
   * People the app can offer to track: the authors of open PRs in the tracked
   * projects, most open PRs first, without the viewer, bots and anyone
   * already tracked. Asks GitHub, so it can take a moment.
   */
  knownContributors(): Promise<KnownContributor[]>;
  /**
   * Asks GitHub, fresh, whether the PR (`owner/name#123`) can be squash-merged
   * now. Call it when the user opens the merge confirmation.
   */
  mergeCheck(id: string): Promise<MergeCheck>;
  /**
   * Squash-merges the PR. The backend re-checks everything first and refuses
   * (rejecting with a readable message) if any review thread is unresolved,
   * GitHub would not allow the merge, or the head commit is no longer
   * `expectedHeadSha`. Irreversible: only call it after the user confirms.
   */
  squashMerge(id: string, expectedHeadSha: string): Promise<void>;
  /**
   * Asks GitHub, fresh, whether the viewer can post the self-review on the PR
   * (`owner/name#123`) and what exactly would be posted. Call it when the
   * user opens the confirmation.
   */
  selfReviewCheck(id: string): Promise<SelfReviewCheck>;
  /**
   * Posts the comment `/self-reviewed <expectedHeadSha>` on the PR as the
   * user. The backend re-checks first and refuses (rejecting with a readable
   * message) if the head commit is no longer `expectedHeadSha`, the PR is
   * closed or a draft, or the user does not hold it. It is the user's own
   * statement on GitHub: only call it after they confirm.
   */
  postSelfReview(id: string, expectedHeadSha: string): Promise<void>;
  /** Opens a URL in the user's browser. */
  openUrl(url: string): Promise<void>;

  // Running a project's example apps.

  /** What can be run for each tracked project. Cheap; ask at start-up and after `setClone` or a project change. */
  runTargets(): Promise<RunProject[]>;
  /**
   * Tells the app where the local clone of `repo` is; null goes back to
   * finding it automatically. Rejects with a readable message when `path` is
   * not a clone of that repository. Resolves to the new `runTargets`.
   */
  setClone(repo: string, path: string | null): Promise<RunProject[]>;
  /** Lets the user pick a folder with the system dialog. Resolves to null when they cancel. */
  pickFolder(): Promise<string | null>;
  /** Sources and devices for running the PR (`owner/name#123`). Rejects with a readable message. */
  runOptions(id: string): Promise<RunOptions>;
  /**
   * Starts building and opening an example app. Resolves as soon as the job
   * has started; progress arrives through `onRunJob` and `onRunLog`. Rejects
   * with a readable message when another job is running or the request
   * cannot be met.
   */
  runStart(request: RunRequest): Promise<RunJob>;
  /** Stops the running job, if any. The job then arrives as `cancelled`. */
  runCancel(): Promise<void>;
  /** The running job or the last one, or null when none has run since launch. */
  runJob(): Promise<RunJob | null>;
  /** The last lines of that job's log (at most 2000), oldest first. */
  runLog(): Promise<string[]>;
  /** Opens that job's complete log file in the user's default app for text. */
  runOpenLog(): Promise<void>;
  onSnapshot(cb: (snapshot: Snapshot) => void): Promise<Unsubscribe>;
  onSyncState(cb: (state: SyncState) => void): Promise<Unsubscribe>;
  onRunJob(cb: (job: RunJob) => void): Promise<Unsubscribe>;
  onRunLog(cb: (log: RunLog) => void): Promise<Unsubscribe>;
}

/** Command and event names. The Rust side must match these exactly. */
export const COMMANDS = {
  getSnapshot: "get_snapshot",
  getConfig: "get_config",
  refresh: "refresh",
  addProject: "add_project",
  removeProject: "remove_project",
  followPr: "follow_pr",
  unfollowPr: "unfollow_pr",
  listOpenPrs: "list_open_prs",
  trackContributor: "track_contributor",
  untrackContributor: "untrack_contributor",
  knownContributors: "known_contributors",
  mergeCheck: "merge_check",
  squashMerge: "squash_merge",
  selfReviewCheck: "self_review_check",
  postSelfReview: "post_self_review",
  runTargets: "run_targets",
  setClone: "set_clone",
  runOptions: "run_options",
  runStart: "run_start",
  runCancel: "run_cancel",
  runJob: "run_job",
  runLog: "run_log",
  runOpenLog: "run_open_log",
} as const;

export const EVENTS = {
  snapshot: "snapshot",
  syncState: "sync-state",
  runJob: "run-job",
  runLog: "run-log",
} as const;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function createTauriApi(): Api {
  const core = () => import("@tauri-apps/api/core");
  const event = () => import("@tauri-apps/api/event");
  const invoke = async <T>(cmd: string, args?: Record<string, unknown>) =>
    (await core()).invoke<T>(cmd, args);
  const listen = async <T>(name: string, cb: (payload: T) => void) =>
    (await event()).listen<T>(name, (e) => cb(e.payload));

  return {
    getSnapshot: () => invoke<Snapshot | null>(COMMANDS.getSnapshot),
    getConfig: () => invoke<Config>(COMMANDS.getConfig),
    refresh: () => invoke<void>(COMMANDS.refresh),
    addProject: (input) => invoke<Config>(COMMANDS.addProject, { input }),
    removeProject: (repo) => invoke<Config>(COMMANDS.removeProject, { repo }),
    followPr: (input) => invoke<Config>(COMMANDS.followPr, { input }),
    unfollowPr: (id) => invoke<Config>(COMMANDS.unfollowPr, { id }),
    listOpenPrs: (repo) => invoke<PrSummary[]>(COMMANDS.listOpenPrs, { repo }),
    trackContributor: (input) => invoke<Config>(COMMANDS.trackContributor, { input }),
    untrackContributor: (login) => invoke<Config>(COMMANDS.untrackContributor, { login }),
    knownContributors: () => invoke<KnownContributor[]>(COMMANDS.knownContributors),
    mergeCheck: (id) => invoke<MergeCheck>(COMMANDS.mergeCheck, { id }),
    squashMerge: (id, expectedHeadSha) => invoke<void>(COMMANDS.squashMerge, { id, expectedHeadSha }),
    selfReviewCheck: (id) => invoke<SelfReviewCheck>(COMMANDS.selfReviewCheck, { id }),
    postSelfReview: (id, expectedHeadSha) => invoke<void>(COMMANDS.postSelfReview, { id, expectedHeadSha }),
    openUrl: async (url) => (await import("@tauri-apps/plugin-opener")).openUrl(url),
    runTargets: () => invoke<RunProject[]>(COMMANDS.runTargets),
    setClone: (repo, path) => invoke<RunProject[]>(COMMANDS.setClone, { repo, path }),
    pickFolder: async () => {
      const picked = await (await import("@tauri-apps/plugin-dialog")).open({ directory: true, multiple: false });
      return typeof picked === "string" ? picked : null;
    },
    runOptions: (id) => invoke<RunOptions>(COMMANDS.runOptions, { id }),
    runStart: (request) => invoke<RunJob>(COMMANDS.runStart, { request }),
    runCancel: () => invoke<void>(COMMANDS.runCancel),
    runJob: () => invoke<RunJob | null>(COMMANDS.runJob),
    runLog: () => invoke<string[]>(COMMANDS.runLog),
    runOpenLog: () => invoke<void>(COMMANDS.runOpenLog),
    onSnapshot: (cb) => listen<Snapshot>(EVENTS.snapshot, cb),
    onSyncState: (cb) => listen<SyncState>(EVENTS.syncState, cb),
    onRunJob: (cb) => listen<RunJob>(EVENTS.runJob, cb),
    onRunLog: (cb) => listen<RunLog>(EVENTS.runLog, cb),
  };
}

/** In a browser, `?snapshot=<url>` shows a captured snapshot instead of the mock data. */
function devSnapshotUrl(): string | undefined {
  if (typeof window === "undefined") return undefined;
  return new URLSearchParams(window.location.search).get("snapshot") ?? undefined;
}

export const api: Api = isTauri() ? createTauriApi() : createMockApi({ snapshotUrl: devSnapshotUrl() });
