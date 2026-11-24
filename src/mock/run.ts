// A believable stand-in for building and opening a project's example apps,
// so the run flow works in a browser and in tests. One job at a time walks
// through its steps on timers and emits the whole job on every change and
// its log in batches, the way the backend does.

import type {
  PullRequest,
  RunDevice,
  RunJob,
  RunLog,
  RunOptions,
  RunPlatform,
  RunPlatformOptions,
  RunProject,
  RunRequest,
  RunSource,
  RunStep,
} from "../types";

export interface RunSimOptions {
  /** Milliseconds each step takes. 0 runs the whole job in a few timer turns. */
  stepMs: number;
  /** Key of a step that fails, e.g. "buildApp". */
  failStep?: string;
  /** Log lines in each batch. Each step emits four batches. */
  linesPerBatch: number;
  /** Platforms that cannot run, with the sentence saying why. */
  unavailable: Partial<Record<RunPlatform, string>>;
  /** Tracked projects, in config order. */
  projects(): string[];
  /** The PR with this id, from the current snapshot. */
  findPr(id: string): PullRequest | undefined;
}

/** Where the app keeps its own checkouts and logs in the mock. */
const HOME = "/Users/you";
const APP_DATA = `${HOME}/Library/Application Support/PR Status`;

/** The project whose example apps the mock can run. */
export const RUNNABLE_REPO = "dashpay/platform";
const AUTO_CLONE = `${HOME}/src/platform`;

/** A fixed folder for the mock system picker. */
export const PICKED_FOLDER = `${HOME}/src/platform-clone`;

const BATCHES_PER_STEP = 4;

const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

const IOS_DEVICES: RunDevice[] = [
  { id: "5C1D7E0A-17PRO", name: "iPhone 17 Pro", detail: "iOS 26.1", kind: "simulator", running: true },
  { id: "0B6F2C9E-17", name: "iPhone 17", detail: "iOS 26.1", kind: "simulator", running: false },
  { id: "8E3A41D2-AIR", name: "iPhone Air", detail: "iOS 26.1", kind: "simulator", running: false },
  { id: "D47C0F13-IPAD", name: "iPad Pro 13-inch (M5)", detail: "iPadOS 26.1", kind: "simulator", running: false },
];

const ANDROID_DEVICES: RunDevice[] = [
  { id: "avd:palmatum_api35", name: "palmatum_api35", detail: "Android 15 · Pixel 9", kind: "emulator", running: true },
  { id: "avd:pixel7_api34", name: "pixel7_api34", detail: "Android 14 · Pixel 7", kind: "emulator", running: false },
];

const STEP_LABELS: Record<RunPlatform, [string, string][]> = {
  ios: [
    ["checkout", "Check out the code"],
    ["buildSdk", "Build the SDK"],
    ["buildApp", "Build the app"],
    ["device", "Start the simulator"],
    ["install", "Install the app"],
    ["open", "Open the app"],
  ],
  android: [
    ["checkout", "Check out the code"],
    ["buildSdk", "Build the SDK"],
    ["buildApp", "Build the app"],
    ["device", "Start the emulator"],
    ["install", "Install the app"],
    ["open", "Open the app"],
  ],
};

const FAILURES: Record<string, string> = {
  checkout: "Could not fetch the pull request head from GitHub.",
  buildSdk: "The SDK did not build: cargo exited with status 101.",
  buildApp: "The app did not build: xcodebuild exited with status 65.",
  device: "The simulator did not start within two minutes.",
  install: "The app could not be installed on the device.",
  open: "The app was installed but did not open.",
};

/** A few plausible log lines for one batch of a step. */
function logLines(platform: RunPlatform, step: string, batch: number, count: number, sha: string): string[] {
  const crates = ["dpp", "drive", "dash-sdk", "rs-sdk-ffi", "platform-version", "grovedb", "dapi-grpc", "simple-signer"];
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    const n = batch * count + i;
    switch (step) {
      case "checkout":
        out.push(
          n === 0
            ? `$ git fetch origin ${sha}`
            : n === 1
              ? `$ git checkout --detach ${sha.slice(0, 12)}`
              : n < 7
                ? `Updating files: ${20 * (n - 1)}% (${842 * (n - 1)}/4210)`
                : n === 7
                  ? `HEAD is now at ${sha.slice(0, 7)}`
                  : `$ git submodule update --init (${n - 7})`,
        );
        break;
      case "buildSdk":
        out.push(`   Compiling ${crates[n % crates.length]} v2.1.0-dev.${(n % 9) + 1} (${HOME}/src/platform/packages/rs-${crates[n % crates.length]})`);
        break;
      case "buildApp":
        out.push(
          platform === "ios"
            ? `CompileSwift normal arm64 SwiftExampleApp/Views/View${n}.swift (in target 'SwiftExampleApp' from project 'SwiftExampleApp')`
            : `> Task :app:${["compileDebugKotlin", "mergeDebugResources", "dexBuilderDebug", "packageDebug"][n % 4]}`,
        );
        break;
      case "device":
        out.push(platform === "ios" ? `xcrun simctl boot … waiting (${n + 1})` : `adb wait-for-device … (${n + 1})`);
        break;
      case "install":
        out.push(platform === "ios" ? `xcrun simctl install booted SwiftExampleApp.app (${n + 1})` : `adb install -r app-debug.apk (${n + 1})`);
        break;
      default:
        out.push(platform === "ios" ? `xcrun simctl launch booted org.dashfoundation.SwiftExampleApp` : `adb shell am start -n org.dash.example/.MainActivity`);
    }
  }
  return out;
}

export interface RunSim {
  runTargets(): RunProject[];
  setClone(repo: string, path: string | null): RunProject[];
  runOptions(id: string): RunOptions;
  runStart(request: RunRequest): RunJob;
  runCancel(): void;
  runJob(): RunJob | null;
  runLog(): string[];
  onRunJob(cb: (job: RunJob) => void): () => void;
  onRunLog(cb: (log: RunLog) => void): () => void;
}

export function createRunSim(options: RunSimOptions): RunSim {
  const clones = new Map<string, string>();
  let job: RunJob | null = null;
  let log: string[] = [];
  let nextId = 1;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const jobListeners = new Set<(job: RunJob) => void>();
  const logListeners = new Set<(log: RunLog) => void>();

  const nowIso = () => new Date().toISOString();
  const emitJob = () => {
    if (!job) return;
    const copy = clone(job);
    for (const cb of jobListeners) cb(copy);
  };
  const emitLog = (lines: string[]) => {
    if (!job) return;
    log = [...log, ...lines].slice(-2000);
    for (const cb of logListeners) cb({ jobId: job.jobId, lines });
  };

  const projectFor = (repo: string): RunProject => {
    const set = clones.get(repo);
    const clonePath = set ?? (repo === RUNNABLE_REPO ? AUTO_CLONE : null);
    const platforms: RunPlatform[] = clonePath && repo === RUNNABLE_REPO ? ["ios", "android"] : [];
    return { repo, clonePath, cloneIsSet: set != null, platforms };
  };
  const runTargets = () => options.projects().map(projectFor);

  const platformOptions = (platform: RunPlatform): RunPlatformOptions => ({
    platform,
    unavailable: options.unavailable[platform] ?? null,
    devices: options.unavailable[platform] ? [] : platform === "ios" ? IOS_DEVICES : ANDROID_DEVICES,
  });

  const sourcesFor = (pr: PullRequest): RunSource[] => {
    const slug = pr.repo.replace("/", "-");
    const sources: RunSource[] = [
      { kind: "prHead", path: `${APP_DATA}/checkouts/${slug}`, branch: null, sha: pr.headSha, atPrHead: true, dirty: false },
    ];
    if (!pr.headBranch) return sources;
    // Two mock PRs have a worktree of the user's: one clean at the head, one
    // behind it with uncommitted changes.
    if (pr.number === 6033) {
      sources.push({
        kind: "worktree",
        path: `${HOME}/src/platform-worktrees/${pr.headBranch.replace(/\//g, "-")}`,
        branch: pr.headBranch,
        sha: pr.headSha,
        atPrHead: true,
        dirty: false,
      });
    } else if (pr.number === 6034) {
      sources.push({
        kind: "worktree",
        path: `${HOME}/.t3/worktrees/platform/t3code-6034-address-pastaclaw-review-of-0x-text-values`,
        branch: pr.headBranch,
        sha: "def5678a9b0c1d2e3f405162738495a6b7c8d9e0",
        atPrHead: false,
        dirty: true,
      });
    }
    return sources;
  };

  const advance = (batch: number) => {
    if (!job || job.state !== "running") return;
    const i = job.steps.findIndex((s) => s.status === "running");
    if (i < 0) return;
    const step = job.steps[i];
    emitLog(logLines(job.platform, step.key, batch, options.linesPerBatch, job.sha ?? ""));
    if (batch < BATCHES_PER_STEP - 1) {
      timer = setTimeout(() => advance(batch + 1), options.stepMs / BATCHES_PER_STEP);
      return;
    }
    const at = nowIso();
    if (step.key === options.failStep) {
      const error =
        step.key === "buildApp" && job.platform === "android"
          ? "The app did not build: Gradle exited with status 1."
          : (FAILURES[step.key] ?? "The step failed.");
      emitLog([`error: ${error}`]);
      job.steps[i] = { ...step, status: "failed", finishedAt: at };
      job = { ...job, state: "failed", finishedAt: at, error };
      timer = null;
      emitJob();
      return;
    }
    job.steps[i] = { ...step, status: "done", finishedAt: at };
    const next = job.steps[i + 1];
    if (next) {
      job.steps[i + 1] = { ...next, status: "running", startedAt: at };
      emitJob();
      timer = setTimeout(() => advance(0), options.stepMs / BATCHES_PER_STEP);
    } else {
      job = { ...job, state: "succeeded", finishedAt: at };
      timer = null;
      emitJob();
    }
  };

  return {
    runTargets,
    setClone(repo, path) {
      if (path == null) clones.delete(repo);
      else {
        if (path.includes("nope")) throw new Error(`${path} is not a clone of ${repo}.`);
        clones.set(repo, path);
      }
      return runTargets();
    },
    runOptions(id) {
      const pr = options.findPr(id);
      if (!pr) throw new Error(`${id} is not in your list any more.`);
      const project = projectFor(pr.repo);
      if (!project.clonePath) throw new Error(`No local clone of ${pr.repo} was found. Set one in the project's menu.`);
      if (project.platforms.length === 0) throw new Error(`The clone of ${pr.repo} has no example apps.`);
      return {
        id,
        clonePath: project.clonePath,
        sources: sourcesFor(pr),
        platforms: project.platforms.map(platformOptions),
      };
    },
    runStart(request) {
      if (job?.state === "running") throw new Error("Another run is in progress. Cancel it or wait for it to finish.");
      const pr = options.findPr(request.id);
      if (!pr) throw new Error(`${request.id} is not in your list any more.`);
      const platform = platformOptions(request.platform);
      if (platform.unavailable) throw new Error(platform.unavailable);
      const device = platform.devices.find((d) => d.id === request.deviceId);
      if (!device) throw new Error("That device is no longer available. Choose another.");
      const source = sourcesFor(pr).find((s) =>
        request.sourceKind === "prHead" ? s.kind === "prHead" : s.kind === "worktree" && s.path === request.sourcePath,
      );
      if (!source) throw new Error("That worktree is no longer there.");
      const at = nowIso();
      const steps: RunStep[] = STEP_LABELS[request.platform].map(([key, label], i) => ({
        key,
        label,
        status: i === 0 ? "running" : "pending",
        startedAt: i === 0 ? at : null,
        finishedAt: null,
      }));
      const jobId = nextId++;
      job = {
        jobId,
        prId: request.id,
        platform: request.platform,
        deviceName: device.name,
        sourceKind: request.sourceKind,
        sha: source.sha,
        state: "running",
        steps,
        startedAt: at,
        finishedAt: null,
        error: null,
        logPath: `${HOME}/Library/Logs/PR Status/run-${jobId}.log`,
      };
      log = [];
      emitJob();
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => advance(0), options.stepMs / BATCHES_PER_STEP);
      return clone(job);
    },
    runCancel() {
      if (!job || job.state !== "running") return;
      if (timer) clearTimeout(timer);
      timer = null;
      const at = nowIso();
      job = {
        ...job,
        state: "cancelled",
        finishedAt: at,
        // As the backend does: the interrupted step did not fail, so it is pending again, with its times kept.
        steps: job.steps.map((s) => (s.status === "running" ? { ...s, status: "pending", finishedAt: at } : s)),
      };
      emitLog(["Cancelled by you."]);
      emitJob();
    },
    runJob: () => (job ? clone(job) : null),
    runLog: () => [...log],
    onRunJob(cb) {
      jobListeners.add(cb);
      return () => void jobListeners.delete(cb);
    },
    onRunLog(cb) {
      logListeners.add(cb);
      return () => void logListeners.delete(cb);
    },
  };
}
