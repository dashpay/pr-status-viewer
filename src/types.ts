// The IPC contract: everything the UI receives from the backend.
//
// Mirrors `src-tauri/src/model.rs` by hand. Change both files together, and
// never change either without the coordinator's say-so: four sessions build
// against them in parallel.
//
// Timestamps are RFC 3339 strings in UTC.

/** GitHub login of the review bot. */
export const BOT_LOGIN = "thepastaclaw";

/** One complete picture of everything the UI shows. Replaced whole on every sync. */
export interface Snapshot {
  generatedAt: string;
  /** null until GitHub has told us who the user is. */
  viewer: Viewer | null;
  /**
   * True when no usable GitHub token could be found: the user has to sign in.
   * False for every other failure (offline, rate limit, ...).
   */
  signInRequired: boolean;
  /**
   * True when T3 Code's threads were read (through its `t3` command), so
   * `t3Threads` is meaningful. False when T3 Code is not installed, not
   * running, or could not be read.
   */
  t3Available: boolean;
  /** One entry per configured project, in config order. */
  projects: ProjectSnapshot[];
  github: SourceHealth;
  feed: FeedHealth;
}

export interface Viewer {
  login: string;
  avatarUrl: string | null;
}

export interface SourceHealth {
  /** False when the most recent fetch failed. */
  ok: boolean;
  lastSuccessAt: string | null;
  /** Human-readable reason for the most recent failure. */
  error: string | null;
}

/** Health of the review bot's public status feed, plus its headline numbers. */
export interface FeedHealth extends SourceHealth {
  /** When the bot generated the data. */
  dataAsOf: string | null;
  running: number;
  queued: number;
  medianReviewMinutes: number | null;
  /** True when the bot reports it is running on stand-in models. */
  degraded: boolean;
}

export interface ProjectSnapshot {
  /** `owner/name`. */
  repo: string;
  /** Total open PRs in the repo, not only the ones listed in `prs`. */
  openCount: number;
  /**
   * The viewer's own open PRs, the PRs they follow, and the open PRs of the
   * contributors they track. Unordered. A tracked contributor's PR has
   * `isMine` and `isFollowed` false unless it is also followed.
   */
  prs: PullRequest[];
  /** Set when this project could not be fetched. */
  error: string | null;
}

export interface PullRequest {
  /** `owner/name#123`. Unique across the snapshot. */
  id: string;
  repo: string;
  number: number;
  title: string;
  url: string;
  author: string;
  authorAvatarUrl: string | null;
  /** Title of the PR's milestone as GitHub shows it, e.g. "v5.1.0". null when it has none. */
  milestone: string | null;
  isDraft: boolean;
  /** Authored by the viewer. */
  isMine: boolean;
  /** In the config's followed list. */
  isFollowed: boolean;
  additions: number;
  deletions: number;
  createdAt: string;
  updatedAt: string;
  headSha: string;
  /** Name of the PR's head branch, e.g. "fix/sdk-timeout". null when GitHub did not report one. */
  headBranch: string | null;
  /** CI on the head commit, not counting the bot's own check run. */
  ci: CiState;
  reviewDecision: ReviewDecision | null;
  /** Human reviewers only: latest review per person, plus pending requests. */
  humans: HumanReview[];
  bot: BotReview;
  /**
   * How many reviews the review bot has posted on this PR: its reviews that
   * carry its round marker. Its replies in threads and its "CI failed after
   * approval" follow-ups are not counted. Null until the PR's reviews have
   * been read.
   */
  rounds: number | null;
  /**
   * What the repo's "PR Hygiene" gate says happens next. Null when the repo
   * does not use the gate or it has said nothing yet.
   */
  nextStep: NextStep | null;
  /**
   * When the most recent of the counted review rounds was submitted: the later
   * of thepastaclaw's and CodeRabbit's last rounds on this PR.
   */
  lastReviewAt: string | null;
  /**
   * The findings on this PR by severity, counting only the last review round
   * of each reviewer (thepastaclaw and CodeRabbit).
   */
  findingCounts: FindingCounts;
  /** The branch the PR merges into. */
  baseBranch: string;
  /**
   * The number of the PR this one is stacked on: the PR in the same project,
   * in this snapshot, whose head branch (not from a fork) is this PR's base
   * branch. null when there is none, or when that branch is shared rather
   * than a stack. The list shows a stacked PR under its parent.
   */
  stackedOn: number | null;
  /**
   * Review threads ("conversations") not yet resolved, outdated ones included.
   * Top-level PR comments cannot be resolved and do not count.
   */
  unresolvedThreads: number;
  /** Whether the squash-merge button is enabled, and if not, why. */
  merge: MergeReadiness;
  /** GitHub reports merge conflicts with the base branch. False while GitHub has not worked it out yet. */
  hasConflicts: boolean;
  /** Whether the viewer can post the self-review the PR Hygiene gate asks for, and whether the gate is asking now. */
  selfReview: SelfReview;
  /** T3 Code threads tied to this PR, working ones first. Empty when there are none. */
  t3Threads: T3Thread[];
  /** Traits, at most one per kind, GroveDB first. Empty for most pull requests. */
  traits: PrTrait[];
  /** True when the viewer is the one who has to act next. */
  needsMe: boolean;
  /** Short reason, e.g. "Changes requested" or "CI failing". */
  needsMeReason: string | null;
}

/** Something notable a pull request does, reported by a workflow comment. */
export interface PrTrait {
  kind: TraitKind;
  /** One short line for a tooltip, e.g. "5 nodes changed" or "token-history-contract v3: new version". */
  summary: string;
  /** Where to look at it. The first is the one a plain click opens. May be empty. */
  links: TraitLink[];
}
export type TraitKind = "grovedbChange" | "contractChange";
export interface TraitLink {
  /** Short label, e.g. "Structure viewer" or "token-history-contract v3 (compare with v2)". */
  label: string;
  url: string;
}

/** The next step in the PR's workflow, read from the "PR Hygiene" status. */
export interface NextStep {
  /** Stage code, lower case, e.g. "waiting-self-review". */
  stage: string;
  /** Short display text, e.g. "Your move". */
  label: string;
  /** The sentence after the stage code, as the gate wrote it. */
  detail: string | null;
  actor: NextActor;
}

/** Who the next step is waiting on. */
export type NextActor = "author" | "bots" | "reviewers" | "ci" | "merge" | "unknown";

export type CiState = "passing" | "failing" | "pending" | "none";
export type ReviewDecision = "approved" | "changesRequested" | "reviewRequired";

export interface HumanReview {
  login: string;
  avatarUrl: string | null;
  state: HumanReviewState;
}

/** `requested` means a review was asked for and nothing is submitted yet. */
export type HumanReviewState =
  | "approved"
  | "changesRequested"
  | "commented"
  | "dismissed"
  | "requested";

/** Where the bot's review of the PR's head commit stands. */
export type BotReview =
  | BotNone
  | BotDraft
  | BotQueued
  | BotReviewing
  | BotApproved
  | BotChangesRequested
  | BotCommented
  | BotOther;

/** The bot has no check on this commit (repo not covered, or not yet seen). */
export interface BotNone {
  state: "none";
}

/** The bot skipped the PR because it is a draft. */
export interface BotDraft {
  state: "draft";
}

export interface BotQueued {
  state: "queued";
  /** 1-based position in the bot's queue, when the feed lists the PR. */
  position: number | null;
  /** When the review can start. */
  startsAt: string | null;
  /** Our estimate from the bot's median review time. Not the bot's own figure. */
  finishesAt: string | null;
  /** Why it is waiting, e.g. "debounce (more pushes may follow)". */
  reason: string | null;
  priority: boolean;
}

export interface BotReviewing {
  state: "reviewing";
  /** Feed phase key, e.g. "phase1". */
  phase: string;
  /** Display form, e.g. "Phase 1". */
  phaseLabel: string;
  /** 0 to 1, when the feed reports it. */
  progress: number | null;
  /** The bot's own estimate. */
  finishesAt: string | null;
  startedAt: string | null;
  /** The bot says this run is taking longer than usual. */
  overdue: boolean;
  /** Triage tier, e.g. "normal", "critical". */
  tier: string | null;
  priority: boolean;
  /** Seconds the PR sat in the queue before the run started. */
  waitedSeconds: number | null;
  /** The whole pipeline in order: done, then the running step, then pending. */
  steps: ReviewStep[];
  findings: Findings | null;
}

export interface BotApproved {
  state: "approved";
  at: string | null;
}

export interface BotChangesRequested {
  state: "changesRequested";
  at: string | null;
}

/** Reviewed with comments, no changes requested. */
export interface BotCommented {
  state: "commented";
  at: string | null;
}

/** A check-run state the backend does not recognise; show `title` as is. */
export interface BotOther {
  state: "other";
  title: string;
}

export interface ReviewStep {
  /** Feed step name, e.g. "phase1", "verify1", "publish". */
  key: string;
  /** Display form, e.g. "Phase 1". */
  label: string;
  status: StepStatus;
  /** Extra progress text, e.g. "0 of 2 lanes". */
  detail: string | null;
}

export type StepStatus = "done" | "running" | "pending" | "failed";

export interface Findings {
  blocking: number;
  suggestion: number;
  nitpick: number;
}

/**
 * The findings on a PR by severity, counting only the last review round of
 * each reviewer (thepastaclaw and CodeRabbit).
 */
export interface FindingCounts {
  blocking: SeverityCount;
  suggestion: SeverityCount;
  nitpick: SeverityCount;
}

/**
 * How the findings of one severity stand in the last review round of each
 * reviewer (thepastaclaw and CodeRabbit).
 * Open = total - agreed - notAgreed - awaiting.
 */
export interface SeverityCount {
  total: number;
  /**
   * Settled with the reviewer's agreement: the bot marked the finding
   * resolved, withdrawn or no longer applicable; CodeRabbit marked it addressed.
   */
  agreed: number;
  /**
   * Closed without agreement: disputed or deferred, or the thread was resolved
   * on GitHub while the finding still stands although its reviewer's last
   * round was on the PR's current head (it saw the code and did not lift it).
   */
  notAgreed: number;
  /**
   * Awaiting re-review: the thread was resolved on GitHub while the finding
   * still stands, and its reviewer's last round was on an older commit than
   * the PR's current head, so the fix is not re-checked yet. A snapshot from
   * an older backend may lack it; read it as 0.
   */
  awaiting: number;
  /** How many of `total` are CodeRabbit's findings (the rest are the review bot's). */
  coderabbit: number;
}

/** Whether a PR can be squash-merged from the app right now. */
export interface MergeReadiness {
  /** True only when every review thread is resolved AND GitHub would allow a squash merge by the viewer. */
  ready: boolean;
  /** When not ready: the first thing in the way, e.g. "3 unresolved comments". */
  reason: string | null;
}

/** A T3 Code thread tied to a pull request. */
export interface T3Thread {
  threadId: string;
  title: string;
  /**
   * `working`: an agent run is in flight right now. `waiting`: the thread's
   * turn has ended while it waits on something it started (a pull request
   * watch, a background command, a delegated task) and it resumes on its own.
   * `idle`: the thread is open but nothing is running or pending.
   */
  state: "working" | "waiting" | "idle";
  /** When the thread last ran or changed, if known. */
  lastActiveAt: string | null;
  /**
   * T3 Code watches this pull request for the thread (CI, reviews, merge) and
   * wakes the thread when it changes. A snapshot from an older backend may
   * lack it; read it as false.
   */
  watching: boolean;
}

/**
 * The self-review attestation of the PR Hygiene gate: whoever holds a PR (its
 * author, or anyone it is assigned to) states that they have read the final
 * diff themselves, by commenting `/self-reviewed <head commit>`.
 */
export interface SelfReview {
  state: SelfReviewState;
  /** For `posted`: when the app posted it. */
  postedAt: string | null;
}

/**
 * `unavailable`: nothing to offer (the viewer does not hold the PR, it is a draft, the repo has no gate, or the gate is past this step).
 * `possible`: the viewer could post it, but the gate is not asking for it right now.
 * `wanted`: the gate says the next step is to post it.
 * `posted`: the app posted it for the current head commit and the gate has not moved on yet (it re-evaluates a minute or two later).
 */
export type SelfReviewState = "unavailable" | "possible" | "wanted" | "posted";

/** A fresh answer from GitHub to "can I post the self-review on this PR now?". */
export interface SelfReviewCheck {
  /** `owner/name#123`. */
  id: string;
  /** The commit the self-review would cover. Pass it back to `postSelfReview`. */
  headSha: string;
  /** Exactly what would be posted: `/self-reviewed <headSha>`. */
  comment: string;
  canPost: boolean;
  /** When `canPost` is false: why, as a sentence. */
  reason: string | null;
  /** When the viewer last posted a self-review naming this commit, if they have. Posting again is allowed. */
  alreadyPostedAt: string | null;
  /** What the gate says right now, e.g. "Bots are done - your move: post /self-reviewed.". null when it says nothing. */
  gate: string | null;
  /** The PR's "Files changed" page, for a last look at the diff. */
  diffUrl: string;
}

/** A fresh answer from GitHub to "can this PR be squash-merged now?". */
export interface MergeCheck {
  /** `owner/name#123`. */
  id: string;
  /** The commit that would be merged. Pass it back to `squashMerge`. */
  headSha: string;
  baseBranch: string;
  /** The title GitHub will give the squash commit. */
  commitTitle: string;
  unresolvedThreads: number;
  merge: MergeReadiness;
}

/** A light row for the "browse a project's open PRs and follow some" list. */
export interface PrSummary {
  id: string;
  repo: string;
  number: number;
  title: string;
  url: string;
  author: string;
  isDraft: boolean;
  isMine: boolean;
  isFollowed: boolean;
  updatedAt: string;
}

/** What the user has chosen to track. */
export interface Config {
  /** Repos as `owner/name`, in the order the user added them. */
  projects: string[];
  /** PR ids (`owner/name#123`) the user follows. */
  followed: string[];
  /**
   * GitHub logins of the contributors the user tracks, in the order they were
   * added, in GitHub's own casing. The snapshot lists their open PRs in the
   * tracked projects alongside the user's.
   */
  contributors: string[];
}

/** Someone the app can offer to track: an author of open PRs in the tracked projects. */
export interface KnownContributor {
  login: string;
  avatarUrl: string | null;
  /** Their open PRs across the tracked projects. */
  openPrs: number;
}

export interface SyncState {
  syncing: boolean;
}

// ---------------------------------------------------------------------------
// Running a project's example apps.
// ---------------------------------------------------------------------------

/** Which example app to build and open. */
export type RunPlatform = "ios" | "android";

/** What can be run for one tracked project. One entry per configured project, in config order. */
export interface RunProject {
  /** `owner/name`. */
  repo: string;
  /** The local clone of the repo, when one is known. */
  clonePath: string | null;
  /** True when the user chose `clonePath`; false when it was found automatically (through T3 Code's projects). */
  cloneIsSet: boolean;
  /** The example apps found in the clone. Empty when there is no clone or it has none: offer nothing to run. */
  platforms: RunPlatform[];
}

/** Everything the "run" dialog needs for one PR. Ask fresh each time the dialog opens. */
export interface RunOptions {
  /** `owner/name#123`. */
  id: string;
  clonePath: string;
  /** Where the code can come from. Never empty; the first is the default. */
  sources: RunSource[];
  /** One entry per example app the project has. */
  platforms: RunPlatformOptions[];
}

/**
 * `prHead`: the PR's head commit, in a checkout the app keeps for itself.
 * `worktree`: one of the user's own worktrees that is on the PR's branch, built in place with whatever is in it.
 */
export type RunSourceKind = "prHead" | "worktree";

/** One place the code to build can come from. */
export interface RunSource {
  kind: RunSourceKind;
  /** The worktree's directory; for `prHead`, the app's own checkout. */
  path: string;
  /** The branch checked out there. null for `prHead` and for a detached worktree. */
  branch: string | null;
  /** The commit that would be built: the PR's head for `prHead`, the worktree's HEAD otherwise. */
  sha: string;
  /** The commit is the PR's head commit. Always true for `prHead`. */
  atPrHead: boolean;
  /** The worktree has uncommitted changes. Always false for `prHead`. */
  dirty: boolean;
}

export interface RunPlatformOptions {
  platform: RunPlatform;
  /** null when it can run. Otherwise why it cannot, as a sentence, e.g. "Xcode is not installed.". */
  unavailable: string | null;
  /** Where it can run, the best choice first (running ones first). */
  devices: RunDevice[];
}

/** `simulator`: iOS simulator. `emulator`: Android virtual device. `device`: physical Android device over adb. */
export type RunDeviceKind = "simulator" | "emulator" | "device";

export interface RunDevice {
  /** Stable across runs: a simulator's UDID, `avd:<name>` for an emulator, `adb:<serial>` for a physical device. */
  id: string;
  /** e.g. "iPhone 18 Pro Max" or "palmatum_api35". */
  name: string;
  /** e.g. "iOS 27.2". null when nothing useful is known. */
  detail: string | null;
  kind: RunDeviceKind;
  /** Booted or connected right now. */
  running: boolean;
}

/** What the user chose in the run dialog. */
export interface RunRequest {
  /** `owner/name#123`. */
  id: string;
  platform: RunPlatform;
  sourceKind: RunSourceKind;
  /** For `worktree`: the `path` of the chosen `RunSource`. Leave null for `prHead`. */
  sourcePath: string | null;
  /** A `RunDevice.id`. */
  deviceId: string;
}

export type RunState = "running" | "succeeded" | "failed" | "cancelled";

/**
 * One build-and-open run. Only one runs at a time; the last one is kept until
 * the next starts. Arrives whole on the `run-job` event whenever it changes.
 */
export interface RunJob {
  /** Unique within one launch of the app; later jobs have higher ids. */
  jobId: number;
  /** `owner/name#123`. */
  prId: string;
  platform: RunPlatform;
  /** e.g. "iPhone 18 Pro Max". */
  deviceName: string;
  sourceKind: RunSourceKind;
  /** The commit being built, when known. */
  sha: string | null;
  state: RunState;
  /** Every step in order: done, then the running one, then pending. */
  steps: RunStep[];
  startedAt: string;
  finishedAt: string | null;
  /** When `state` is `failed`: one sentence saying what went wrong. */
  error: string | null;
  /** Where the complete log is kept. */
  logPath: string | null;
}

export interface RunStep {
  /** Stable name, e.g. "checkout", "buildSdk", "buildApp", "device", "install", "open". */
  key: string;
  /** Display text, e.g. "Build the SDK". */
  label: string;
  status: StepStatus;
  startedAt: string | null;
  finishedAt: string | null;
}

/** New output of the running job, on the `run-log` event in batches. */
export interface RunLog {
  jobId: number;
  lines: string[];
}
