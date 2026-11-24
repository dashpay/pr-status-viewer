// In-memory stand-in for the backend. Used in a plain browser and in tests.

import type { Api, Unsubscribe } from "../api";
import type { Config, PullRequest, RunPlatform, Snapshot, SyncState } from "../types";
import { MOCK_PEOPLE, mockConfig, mockContributorPrs, mockOpenPrs, mockPeopleCounts, mockSnapshot } from "./snapshot";
import { PICKED_FOLDER, createRunSim } from "./run";

export interface MockApiOptions {
  snapshot?: Snapshot | null;
  config?: Config;
  /** Milliseconds each call takes. 0 in tests. */
  latencyMs?: number;
  /**
   * URL of a snapshot JSON file to show instead of the built-in mock data,
   * e.g. one captured from the real backend with the `snapshot_dump` example.
   */
  snapshotUrl?: string;
  /**
   * Milliseconds each step of a simulated run takes. 0 finishes a run in a
   * few timer turns. Default 2500, or `?runStepMs=` in a browser.
   */
  runStepMs?: number;
  /** Key of the step a simulated run fails at, e.g. "buildApp". Default none, or `?runFail=` in a browser. */
  runFailStep?: string;
  /** Log lines in each batch a simulated run emits (four batches a step). Default 3. */
  runLinesPerBatch?: number;
  /** Platforms that cannot run, with the sentence saying why. */
  runUnavailable?: Partial<Record<RunPlatform, string>>;
  /** When set, every self-review check says it cannot be posted, with this sentence. Or `?selfReviewBlocked=` in a browser. */
  selfReviewBlocked?: string;
  /** When set, the self-review check says the viewer already posted one for the head commit at this time. */
  selfReviewPostedAt?: string;
  /**
   * When true, the first `postSelfReview` is refused as if new commits had
   * just been pushed: the PR's head moves to `SIMULATED_PUSH_SHA` and the
   * snapshot is published, so a fresh check shows the new commit.
   */
  selfReviewPushOnce?: boolean;
  /** When set, `knownContributors` rejects with this sentence. Or `?knownFail=` in a browser. */
  knownContributorsError?: string;
  /** When true, `knownContributors` finds nobody. */
  knownContributorsEmpty?: boolean;
  /** Extra milliseconds `knownContributors` takes, to show its loading state. Or `?knownMs=` in a browser. */
  knownContributorsDelayMs?: number;
}

/** The head a PR moves to when `selfReviewPushOnce` simulates a push. */
export const SIMULATED_PUSH_SHA = "9e7d4c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d";

/** What the backend says when the head moved between the check and the post. */
export const HEAD_MOVED_MESSAGE = "New commits were pushed. Review them, then try again.";

/** Dev aid: `?runStepMs=` and `?runFail=` steer the simulated run in a browser. */
function urlParam(name: string): string | undefined {
  if (typeof window === "undefined") return undefined;
  return new URLSearchParams(window.location.search).get(name) ?? undefined;
}

export function createMockApi(options: MockApiOptions = {}): Api {
  let snapshot: Snapshot | null = options.snapshot === undefined ? mockSnapshot() : options.snapshot;
  let config: Config = options.config ?? mockConfig();
  const latency = options.latencyMs ?? 150;
  const snapshotListeners = new Set<(s: Snapshot) => void>();
  const syncListeners = new Set<(s: SyncState) => void>();
  const stepParam = urlParam("runStepMs");
  const stepFromUrl = stepParam != null && Number(stepParam) >= 0 ? Number(stepParam) : undefined;
  const run = createRunSim({
    stepMs: options.runStepMs ?? stepFromUrl ?? 2500,
    failStep: options.runFailStep ?? urlParam("runFail"),
    linesPerBatch: options.runLinesPerBatch ?? 3,
    unavailable: options.runUnavailable ?? {},
    projects: () => config.projects,
    findPr: (id) => snapshot?.projects.flatMap((p) => p.prs).find((x) => x.id === id),
  });

  // A captured snapshot replaces the mock data before the first call answers.
  const captured: Promise<void> = options.snapshotUrl
    ? fetch(options.snapshotUrl)
        .then((r) => r.json() as Promise<Snapshot>)
        .then((loaded) => {
          snapshot = loaded;
          config = {
            projects: loaded.projects.map((p) => p.repo),
            followed: loaded.projects.flatMap((p) => p.prs.filter((x) => x.isFollowed).map((x) => x.id)),
            contributors: [],
          };
        })
        .catch((e) => console.error(`Could not load the snapshot at ${options.snapshotUrl}`, e))
    : Promise.resolve();

  // Tracked contributors' PRs join the snapshot, as the backend's sync would add them.
  const withContributorPrs = (snap: Snapshot, logins: string[]): Snapshot => {
    const extra = logins.flatMap((login) => mockContributorPrs(login));
    return {
      ...snap,
      projects: snap.projects.map((p) => {
        const have = new Set(p.prs.map((x) => x.id));
        return { ...p, prs: [...p.prs, ...extra.filter((x) => x.repo === p.repo && !have.has(x.id))] };
      }),
    };
  };
  if (snapshot && !options.snapshotUrl && config.contributors.length > 0) {
    snapshot = withContributorPrs(snapshot, config.contributors);
  }
  const isTracked = (author: string) => config.contributors.some((c) => c.toLowerCase() === author.toLowerCase());

  const wait = async () => {
    await captured;
    if (latency > 0) await new Promise<void>((r) => setTimeout(r, latency));
  };
  const sub = <T>(set: Set<(v: T) => void>, cb: (v: T) => void): Promise<Unsubscribe> => {
    set.add(cb);
    return Promise.resolve(() => void set.delete(cb));
  };
  const publish = () => {
    if (snapshot) for (const cb of snapshotListeners) cb(snapshot);
  };
  const findPr = (id: string) => snapshot?.projects.flatMap((p) => p.prs).find((x) => x.id === id);
  const updatePr = (id: string, patch: Partial<PullRequest>) => {
    if (!snapshot) return;
    snapshot = {
      ...snapshot,
      projects: snapshot.projects.map((p) => ({ ...p, prs: p.prs.map((x) => (x.id === id ? { ...x, ...patch } : x)) })),
    };
  };
  // Self-reviews posted in this session, by `id@sha`, with when.
  const selfReviewsPosted = new Map<string, string>();
  let pushOnce = options.selfReviewPushOnce ?? false;
  const selfReviewBlocked = options.selfReviewBlocked ?? urlParam("selfReviewBlocked");

  const setFollowed = (id: string, followed: boolean) => {
    if (!snapshot) return;
    snapshot = {
      ...snapshot,
      projects: snapshot.projects.map((p) => ({
        ...p,
        prs: followed
          ? p.prs.map((x) => (x.id === id ? { ...x, isFollowed: true } : x))
          : p.prs.flatMap((x) =>
              x.id !== id ? [x] : x.isMine || isTracked(x.author) ? [{ ...x, isFollowed: false }] : [],
            ),
      })),
    };
  };

  return {
    async getSnapshot() {
      await wait();
      return snapshot;
    },
    async getConfig() {
      await wait();
      return config;
    },
    async refresh() {
      for (const cb of syncListeners) cb({ syncing: true });
      await wait();
      if (snapshot) snapshot = { ...snapshot, generatedAt: new Date().toISOString() };
      publish();
      for (const cb of syncListeners) cb({ syncing: false });
    },
    async addProject(input) {
      await wait();
      const repo = input
        .trim()
        .replace(/^https?:\/\/github\.com\//, "")
        .split("/")
        .slice(0, 2)
        .join("/");
      if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error(`"${input}" is not a repository. Use owner/name.`);
      if (config.projects.some((p) => p.toLowerCase() === repo.toLowerCase())) {
        throw new Error(`${repo} is already in your projects.`);
      }
      config = { ...config, projects: [...config.projects, repo] };
      if (snapshot) {
        snapshot = { ...snapshot, projects: [...snapshot.projects, { repo, openCount: 0, prs: [], error: null }] };
      }
      publish();
      return config;
    },
    async removeProject(repo) {
      await wait();
      config = {
        ...config,
        projects: config.projects.filter((p) => p !== repo),
        followed: config.followed.filter((id) => !id.startsWith(`${repo}#`)),
      };
      if (snapshot) snapshot = { ...snapshot, projects: snapshot.projects.filter((p) => p.repo !== repo) };
      publish();
      return config;
    },
    // Contributors: a simulation. Only `MOCK_PEOPLE` exist on this GitHub.
    async trackContributor(input) {
      await wait();
      const typed = input
        .trim()
        .replace(/^(?:https?:\/\/)?(?:www\.)?github\.com\//i, "")
        .replace(/[/?#].*$/, "")
        .replace(/^@/, "");
      if (!typed) throw new Error("Type a GitHub username.");
      const viewer = snapshot?.viewer?.login;
      if (viewer && typed.toLowerCase() === viewer.toLowerCase()) {
        throw new Error("That is you. Your own pull requests are under Mine.");
      }
      const login = MOCK_PEOPLE.find((p) => p.toLowerCase() === typed.toLowerCase());
      if (!login) throw new Error(`There is no GitHub user named ${typed}.`);
      if (isTracked(login)) return config;
      config = { ...config, contributors: [...config.contributors, login] };
      if (snapshot) snapshot = withContributorPrs(snapshot, [login]);
      publish();
      return config;
    },
    async untrackContributor(login) {
      await wait();
      const gone = (author: string) => author.toLowerCase() === login.toLowerCase();
      config = { ...config, contributors: config.contributors.filter((c) => !gone(c)) };
      if (snapshot) {
        snapshot = {
          ...snapshot,
          projects: snapshot.projects.map((p) => ({
            ...p,
            // A PR the user follows stays, as Following still lists it.
            prs: p.prs.filter((x) => !gone(x.author) || x.isMine || x.isFollowed),
          })),
        };
      }
      publish();
      return config;
    },
    async knownContributors() {
      await wait();
      const delay = options.knownContributorsDelayMs ?? Number(urlParam("knownMs") ?? 0);
      if (delay > 0) await new Promise<void>((r) => setTimeout(r, delay));
      const failure = options.knownContributorsError ?? urlParam("knownFail");
      if (failure) throw new Error(failure);
      if (options.knownContributorsEmpty) return [];
      return mockPeopleCounts()
        .filter((p) => p.openPrs > 0 && !isTracked(p.login))
        .map((p) => ({ login: p.login, avatarUrl: null, openPrs: p.openPrs }));
    },
    async followPr(input) {
      await wait();
      const m = input.trim().match(/^(?:https?:\/\/github\.com\/)?([\w.-]+\/[\w.-]+)(?:#|\/pull\/)(\d+)/);
      if (!m) throw new Error(`"${input}" is not a pull request. Use owner/name#123 or a PR link.`);
      const id = `${m[1]}#${m[2]}`;
      if (!config.followed.includes(id)) config = { ...config, followed: [...config.followed, id] };
      setFollowed(id, true);
      publish();
      return config;
    },
    async unfollowPr(id) {
      await wait();
      config = { ...config, followed: config.followed.filter((f) => f !== id) };
      setFollowed(id, false);
      publish();
      return config;
    },
    async listOpenPrs(repo) {
      await wait();
      return mockOpenPrs(repo).map((p) => ({ ...p, isFollowed: config.followed.includes(p.id) }));
    },
    async mergeCheck(id) {
      await wait();
      const pr = snapshot?.projects.flatMap((p) => p.prs).find((x) => x.id === id);
      if (!pr) throw new Error(`${id} is not in your list any more.`);
      return {
        id,
        headSha: pr.headSha,
        baseBranch: pr.baseBranch,
        commitTitle: `${pr.title} (#${pr.number})`,
        unresolvedThreads: pr.unresolvedThreads,
        merge: pr.merge,
      };
    },
    async squashMerge(id, expectedHeadSha) {
      await wait();
      const pr = snapshot?.projects.flatMap((p) => p.prs).find((x) => x.id === id);
      if (!pr) throw new Error(`${id} is not in your list any more.`);
      if (pr.headSha !== expectedHeadSha) throw new Error("New commits were pushed. Review them, then try again.");
      if (!pr.merge.ready) throw new Error(pr.merge.reason ?? "This pull request cannot be merged yet.");
      // A merged PR is no longer open, so it leaves the snapshot.
      if (snapshot) {
        snapshot = {
          ...snapshot,
          projects: snapshot.projects.map((p) => ({ ...p, prs: p.prs.filter((x) => x.id !== id) })),
        };
      }
      publish();
    },
    // Self-review: a simulation of the backend's checks and the comment it posts.
    async selfReviewCheck(id) {
      await wait();
      const pr = findPr(id);
      if (!pr) throw new Error(`${id} is not in your list any more.`);
      let reason: string | null = selfReviewBlocked ?? null;
      if (reason == null && pr.isDraft) reason = "This pull request is a draft. Mark it ready for review first.";
      if (reason == null && !pr.isMine) reason = "You are not the author of this pull request, and it is not assigned to you.";
      return {
        id,
        headSha: pr.headSha,
        comment: `/self-reviewed ${pr.headSha}`,
        canPost: reason == null,
        reason,
        alreadyPostedAt: selfReviewsPosted.get(`${id}@${pr.headSha}`) ?? options.selfReviewPostedAt ?? null,
        gate: pr.nextStep?.detail ?? null,
        diffUrl: `${pr.url}/files`,
      };
    },
    async postSelfReview(id, expectedHeadSha) {
      await wait();
      const pr = findPr(id);
      if (!pr) throw new Error(`${id} is not in your list any more.`);
      if (pushOnce) {
        pushOnce = false;
        updatePr(id, { headSha: SIMULATED_PUSH_SHA, updatedAt: new Date().toISOString() });
        publish();
        throw new Error(HEAD_MOVED_MESSAGE);
      }
      if (pr.headSha !== expectedHeadSha) throw new Error(HEAD_MOVED_MESSAGE);
      if (selfReviewBlocked) throw new Error(selfReviewBlocked);
      if (pr.isDraft) throw new Error("This pull request is a draft. Mark it ready for review first.");
      if (!pr.isMine) throw new Error("You are not the author of this pull request, and it is not assigned to you.");
      const postedAt = new Date().toISOString();
      selfReviewsPosted.set(`${id}@${pr.headSha}`, postedAt);
      updatePr(id, { selfReview: { state: "posted", postedAt } });
      publish();
    },
    async openUrl(url) {
      window.open(url, "_blank", "noopener");
    },
    // Running example apps: a simulation (mock/run.ts).
    async runTargets() {
      await wait();
      return run.runTargets();
    },
    async setClone(repo, path) {
      await wait();
      return run.setClone(repo, path);
    },
    async pickFolder() {
      await wait();
      return PICKED_FOLDER;
    },
    async runOptions(id) {
      await wait();
      return run.runOptions(id);
    },
    async runStart(request) {
      await wait();
      return run.runStart(request);
    },
    async runCancel() {
      await wait();
      run.runCancel();
    },
    async runJob() {
      await wait();
      return run.runJob();
    },
    async runLog() {
      await wait();
      return run.runLog();
    },
    async runOpenLog() {
      await wait();
    },
    onSnapshot: (cb) => sub(snapshotListeners, cb),
    onSyncState: (cb) => sub(syncListeners, cb),
    onRunJob: (cb) => Promise.resolve(run.onRunJob(cb)),
    onRunLog: (cb) => Promise.resolve(run.onRunLog(cb)),
  };
}
