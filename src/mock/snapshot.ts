// A realistic snapshot for browser development and tests. Timestamps are
// relative to "now" so countdowns and "x min ago" stay believable.

import type {
  BotReview,
  Config,
  FindingCounts,
  MergeReadiness,
  NextActor,
  NextStep,
  PrSummary,
  PrTrait,
  PullRequest,
  SelfReview,
  Snapshot,
  T3Thread,
} from "../types";

const VIEWER = "QuantumExplorer";

/** ISO timestamp `seconds` from now (negative for the past). */
function at(now: number, seconds: number): string {
  return new Date(now + seconds * 1000).toISOString();
}

interface PrSeed {
  repo: string;
  number: number;
  title: string;
  author?: string;
  isDraft?: boolean;
  milestone?: string;
  isFollowed?: boolean;
  additions: number;
  deletions: number;
  /** Seconds ago. */
  updated: number;
  /** Seconds since it was opened; omitted: three days before `updated`, so not recent. */
  created?: number;
  ci: PullRequest["ci"];
  reviewDecision?: PullRequest["reviewDecision"];
  humans?: PullRequest["humans"];
  bot: BotReview;
  rounds?: number;
  nextStep?: NextStep;
  needsMeReason?: string;
  unresolvedThreads?: number;
  /** Omitted: ready to merge. */
  merge?: MergeReadiness;
  /** Omitted: no merge conflicts. */
  hasConflicts?: boolean;
  selfReview?: SelfReview;
  t3Threads?: T3Thread[];
  headBranch?: string;
  /** Omitted: v5.1-dev. A stacked PR's base is its parent's head branch. */
  baseBranch?: string;
  /** The PR in the same project this one is stacked on; omitted: none. */
  stackedOn?: number;
  /** Seconds since the bot's last review; omitted: it never reviewed. */
  lastReview?: number;
  /** Omitted: no findings. */
  findings?: FindingCounts;
  /** Omitted: none. */
  traits?: PrTrait[];
}

/**
 * Findings in the last review round of each reviewer, by severity, each as
 * [total, agreed, not agreed, of which CodeRabbit's (default 0),
 * awaiting re-review (default 0)].
 */
type Seed = [number, number, number, number?, number?];
function counts(blocking: Seed, suggestion: Seed = [0, 0, 0], nitpick: Seed = [0, 0, 0]): FindingCounts {
  const one = ([total, agreed, notAgreed, coderabbit = 0, awaiting = 0]: Seed) => ({
    total,
    agreed,
    notAgreed,
    awaiting,
    coderabbit,
  });
  return { blocking: one(blocking), suggestion: one(suggestion), nitpick: one(nitpick) };
}

const NO_FINDINGS = counts([0, 0, 0]);

/** Not mergeable, for the given reason. */
function blocked(reason: string): MergeReadiness {
  return { ready: false, reason };
}

const READY: MergeReadiness = { ready: true, reason: null };
const NO_PERMISSION = "You do not have permission to merge in this repository.";
const BRANCH_RULES = "GitHub's branch rules require an approving review.";
const CHECKS_RUNNING = "Required checks have not finished.";

function unresolved(n: number): MergeReadiness {
  return blocked(n === 1 ? "1 unresolved comment" : `${n} unresolved comments`);
}

/**
 * A T3 Code thread; `idleFor` is seconds since it last ran, or null while it
 * is working. `waiting`: its turn has ended and it waits on something it
 * started; `watching`: T3 Code watches the pull request for it.
 */
function t3(
  now: number,
  threadId: string,
  title: string,
  idleFor: number | null,
  { waiting = false, watching = false }: { waiting?: boolean; watching?: boolean } = {},
): T3Thread {
  return idleFor == null
    ? { threadId, title, state: "working", lastActiveAt: at(now, 0), watching }
    : { threadId, title, state: waiting ? "waiting" : "idle", lastActiveAt: at(now, -idleFor), watching };
}

/** A "PR Hygiene" next step as the backend would read it. */
function step(stage: string, label: string, actor: NextActor, detail: string | null = null): NextStep {
  return { stage, label, actor, detail };
}

const SELF_REVIEW = "Bots are done - your move: post /self-reviewed.";
const FIX_OR_DISMISS = "Your move: thepastaclaw requested changes on this head; dismiss the review or push a fix.";

const GROVEDB_VIEWER = "https://dashpay.github.io/grovedb-structure-viewer/";
const CONTRACT_VIEWER = "https://dashpay.github.io/contract-visualizer/";

/** A GroveDB structure change, as the platform workflow reports it. */
function grovedb(summary: string, viewer: string | null): PrTrait {
  return {
    kind: "grovedbChange",
    summary,
    links: viewer == null ? [] : [{ label: "Structure viewer", url: `${GROVEDB_VIEWER}${viewer}` }],
  };
}

/** A system contract change: one link per contract, as [contract, version, previous version or null]. */
function contract(summary: string, contracts: [string, number, number | null][]): PrTrait {
  return {
    kind: "contractChange",
    summary,
    links: contracts.map(([name, v, prev]) => ({
      label: prev == null ? `${name} v${v}` : `${name} v${v} (compare with v${prev})`,
      url: `${CONTRACT_VIEWER}?contract=${name}&version=${v}${prev == null ? "" : `&compare=${prev}`}`,
    })),
  };
}

function pr(now: number, s: PrSeed): PullRequest {
  const author = s.author ?? VIEWER;
  return {
    id: `${s.repo}#${s.number}`,
    repo: s.repo,
    number: s.number,
    title: s.title,
    url: `https://github.com/${s.repo}/pull/${s.number}`,
    author,
    authorAvatarUrl: null,
    milestone: s.milestone ?? null,
    isDraft: s.isDraft ?? false,
    isMine: author === VIEWER,
    isFollowed: s.isFollowed ?? false,
    additions: s.additions,
    deletions: s.deletions,
    createdAt: at(now, -(s.created ?? s.updated + 86400 * 3)),
    updatedAt: at(now, -s.updated),
    headSha: "5cc1bd0123d8a0f0c5a56ca6c43ec69f7e1b2c3d",
    headBranch: s.headBranch ?? null,
    ci: s.ci,
    reviewDecision: s.reviewDecision ?? null,
    humans: s.humans ?? [],
    bot: s.bot,
    rounds: s.rounds ?? null,
    nextStep: s.nextStep ?? null,
    lastReviewAt: s.lastReview == null ? null : at(now, -s.lastReview),
    findingCounts: s.findings ?? NO_FINDINGS,
    baseBranch: s.baseBranch ?? "v5.1-dev",
    stackedOn: s.stackedOn ?? null,
    unresolvedThreads: s.unresolvedThreads ?? 0,
    merge: s.merge ?? READY,
    hasConflicts: s.hasConflicts ?? false,
    selfReview: s.selfReview ?? { state: "unavailable", postedAt: null },
    t3Threads: s.t3Threads ?? [],
    traits: s.traits ?? [],
    needsMe: s.needsMeReason != null,
    needsMeReason: s.needsMeReason ?? null,
  };
}

export function mockConfig(): Config {
  return {
    contributors: [],
    projects: ["dashpay/platform", "dashpay/dips", "dashpay/dash-evo-tool", "dashpay/dash"],
    followed: [
      "dashpay/dash-evo-tool#1161",
      "dashpay/dash-evo-tool#1213",
      "dashpay/dash-evo-tool#1219",
      "dashpay/dash-evo-tool#1207",
      "dashpay/dash#7887",
    ],
  };
}

export function mockSnapshot(nowMs: number = Date.now()): Snapshot {
  const now = nowMs;
  const platform: PullRequest[] = [
    pr(now, {
      repo: "dashpay/platform",
      number: 6033,
      // Opened in the last 3 days, for the created recently switch.
      created: 2 * 86400,
      headBranch: "fix/swift-sdk-dpns-contests",
      milestone: "v5.1.0",
      lastReview: 7400,
      rounds: 3,
      // Under review again after the author pushed fixes and resolved every
      // thread (like dashpay/platform#6056): all awaiting re-review.
      findings: counts([5, 0, 0, 0, 5], [5, 0, 0, 0, 5], [2, 1, 0]),
      merge: blocked(CHECKS_RUNNING),
      // Waiting for the bot: the viewer could post the self-review, but the gate is not asking yet.
      selfReview: { state: "possible", postedAt: null },
      title: "fix(swift-sdk)!: list DPNS contests and query one contest's vote state in SwiftExampleApp",
      additions: 1132,
      deletions: 329,
      updated: 70,
      ci: "pending",
      bot: {
        state: "reviewing",
        phase: "phase1",
        phaseLabel: "Phase 1",
        progress: 0.16,
        finishesAt: at(now, 1196),
        startedAt: at(now, -227),
        overdue: false,
        tier: "normal",
        priority: true,
        waitedSeconds: 16,
        steps: [
          { key: "triage", label: "Triage", status: "done", detail: null },
          { key: "phase1", label: "Phase 1", status: "running", detail: "0 of 2 lanes" },
          { key: "verify1", label: "Verify", status: "pending", detail: null },
          { key: "gate", label: "Gate", status: "pending", detail: null },
          { key: "phase2", label: "Phase 2", status: "pending", detail: null },
          { key: "verify2", label: "Verify 2", status: "pending", detail: null },
          { key: "publish", label: "Publish", status: "pending", detail: null },
        ],
        findings: null,
      },
      nextStep: step(
        "waiting-bots",
        "Waiting for bots",
        "bots",
        "Waiting for bot review - thepastaclaw not yet. Wait for the missing reviews, or a writer can post /skip-bots to proceed without them.",
      ),
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 6038,
      // Opened in the last 3 days, for the created recently switch.
      created: 20 * 3600,
      headBranch: "feat/state-transition-proofs",
      traits: [grovedb("5 nodes changed", "?base=083c865a&head=dashpay%2Fplatform%405acaa407")],
      milestone: "v5.1.0",
      lastReview: 620,
      // Everything settled with the bot's agreement.
      findings: counts([2, 2, 0], [4, 4, 0], [3, 3, 0]),
      merge: blocked(BRANCH_RULES),
      t3Threads: [
        t3(now, "t3-6038-ci", "Watch CI on the state transition proofs", 540, { waiting: true, watching: true }),
        t3(now, "t3-6038-proofs", "Self-review the state transition proofs", 7400),
      ],
      selfReview: { state: "wanted", postedAt: null },
      title: "feat(platform): prove the state before and after the block of a state transition",
      additions: 11623,
      deletions: 907,
      updated: 620,
      ci: "passing",
      reviewDecision: "reviewRequired",
      bot: { state: "approved", at: at(now, -620) },
      rounds: 6,
      nextStep: step("waiting-self-review", "Your move", "author", SELF_REVIEW),
      needsMeReason: "Self-review needed",
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 6022,
      // Opened in the last 3 days, for the created recently switch.
      created: 60 * 3600,
      headBranch: "fix/sdk-document-history-v15",
      // The workflow said the structure changed but gave no viewer link.
      traits: [grovedb("Structure description changed", null)],
      milestone: "v5.2.0",
      lastReview: 3180,
      // Suggestions only, one agreed.
      findings: counts([0, 0, 0], [3, 1, 0], [1, 0, 0]),
      unresolvedThreads: 2,
      merge: unresolved(2),
      selfReview: { state: "wanted", postedAt: null },
      t3Threads: [
        t3(now, "t3-6022-history", "Read document history below protocol 15", 90000),
        t3(now, "t3-6022-ci", "Fix the failing SDK tests", null),
      ],
      title: "fix(sdk, drive-abci): read document history on a network below protocol version 15",
      additions: 1111,
      deletions: 192,
      updated: 3180,
      ci: "failing",
      bot: { state: "commented", at: at(now, -3180) },
      nextStep: step("waiting-self-review", "Your move", "author", SELF_REVIEW),
      needsMeReason: "CI failing",
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 6034,
      headBranch: "fix/sdk-0x-text-values",
      traits: [
        grovedb("2 nodes changed", "?base=5cc1bd01&head=dashpay%2Fplatform%40a91f03e2"),
        contract("dpns-contract v2: contested index changed", [["dpns-contract", 2, 1]]),
      ],
      milestone: "v5.1.0",
      lastReview: 9180,
      // Nothing settled yet.
      findings: counts([3, 0, 0], [2, 0, 0]),
      unresolvedThreads: 3,
      merge: unresolved(3),
      t3Threads: [t3(now, "t3-6034-review", "Address PastaClaw's review of 0x text values", null)],
      // Conflicts too; the merge reason names the unresolved comments first, as the backend does.
      hasConflicts: true,
      title: "fix(sdk)!: pass contested index text values starting with 0x as text",
      additions: 1332,
      deletions: 350,
      updated: 9180,
      ci: "failing",
      bot: { state: "changesRequested", at: at(now, -9180) },
      nextStep: step("waiting-self-review", "Your move", "author", FIX_OR_DISMISS),
      needsMeReason: "Changes requested",
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 6023,
      milestone: "v5.2.0",
      lastReview: 9600,
      // Blockers only.
      findings: counts([1, 0, 0]),
      unresolvedThreads: 1,
      merge: unresolved(1),
      title: 'fix(kotlin-example-app): connect to a devnet by name and stop showing "null" ban reasons',
      additions: 1414,
      deletions: 81,
      updated: 9600,
      ci: "failing",
      bot: { state: "changesRequested", at: at(now, -9600) },
      nextStep: step("waiting-self-review", "Your move", "author", FIX_OR_DISMISS),
      needsMeReason: "Changes requested",
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 6002,
      // Opened in the last 3 days, for the created recently switch.
      created: 2 * 86400,
      milestone: "v6.0.0",
      lastReview: 45200,
      // Reviewed, nothing found, then turned back into a draft; the bot
      // skips the new head.
      isDraft: true,
      merge: blocked("This pull request is a draft."),
      title: "fix(dashmate): run an added local node on its network's Core image",
      additions: 100,
      deletions: 7,
      updated: 45200,
      ci: "passing",
      bot: { state: "draft" },
      nextStep: step("draft", "Draft", "author", "The pull request is a draft; mark it ready for review to continue."),
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 5750,
      // Stacked on #5749 (as on GitHub), and #5967 on this one: a stack three deep.
      stackedOn: 5749,
      baseBranch: "feat/random-document-properties",
      traits: [contract("token-history-contract v3: new version", [["token-history-contract", 3, null]])],
      milestone: "v5.1.0",
      lastReview: 259200,
      // All settled, one blocker closed without the bot's agreement.
      findings: counts([1, 0, 1], [2, 1, 1], [1, 1, 0]),
      t3Threads: [t3(now, "t3-5750-rebase", "Rebase random token amounts on v5.1-dev", 3 * 86400, { watching: true })],
      title: "feat(platform)!: random token transfer and mint amounts",
      additions: 5596,
      deletions: 178,
      updated: 259200,
      ci: "passing",
      reviewDecision: "approved",
      humans: [
        { login: "shumkov", avatarUrl: null, state: "approved" },
        { login: "lklimek", avatarUrl: null, state: "requested" },
      ],
      bot: { state: "approved", at: at(now, -259200) },
      nextStep: step("ready-to-merge", "Ready to merge", "merge"),
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 5968,
      milestone: "v5.2.0",
      lastReview: 45100,
      // Mixed: agreed, not agreed and open. Turned into a draft after the
      // review, to work through it.
      findings: counts([4, 1, 1], [6, 2, 3], [2, 0, 0]),
      isDraft: true,
      unresolvedThreads: 4,
      merge: blocked("This pull request is a draft."),
      title: "feat(platform-wallet)!: apps show the shielded balance the wallet announces, without identities",
      additions: 5438,
      deletions: 359,
      updated: 45100,
      ci: "failing",
      bot: { state: "changesRequested", at: at(now, -45100) },
      nextStep: step(
        "waiting-self-review",
        "Your move",
        "author",
        "Your move: thepastaclaw left review threads unresolved; resolve them.",
      ),
      needsMeReason: "Changes requested",
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 5967,
      stackedOn: 5750,
      baseBranch: "feat/random-token-amounts",
      // A draft the bot never reviewed.
      merge: blocked("This pull request is a draft."),
      title: "feat(sdk): host apps can prepare an identity's shielded account",
      isDraft: true,
      additions: 787,
      deletions: 0,
      updated: 351000,
      ci: "failing",
      bot: { state: "draft" },
      nextStep: step("draft", "Draft", "author", "The pull request is a draft; mark it ready for review to continue."),
    }),
    pr(now, {
      repo: "dashpay/platform",
      number: 5749,
      traits: [
        contract("3 system contracts changed", [
          ["dpns-contract", 2, 1],
          ["dashpay-contract", 2, 1],
          ["token-history-contract", 3, 2],
        ]),
      ],
      lastReview: 364000,
      // A huge change: large numbers in one round, CodeRabbit's included.
      findings: counts([14, 11, 2, 5], [118, 104, 9, 76], [140, 121, 9, 98]),
      merge: blocked("Merge conflicts with v5.1-dev."),
      hasConflicts: true,
      title: "feat!: sys.random.uniform document properties",
      additions: 4122,
      deletions: 171,
      updated: 364000,
      ci: "failing",
      reviewDecision: "reviewRequired",
      bot: { state: "changesRequested", at: at(now, -364000) },
      rounds: 30,
      nextStep: step(
        "needs-rebase",
        "Needs rebase",
        "unknown",
        "The branch is behind v3.1-dev; rebase it to continue.",
      ),
      needsMeReason: "Changes requested",
    }),
  ];

  const dips: PullRequest[] = [
    pr(now, {
      repo: "dashpay/dips",
      number: 195,
      merge: blocked(BRANCH_RULES),
      title: "docs: add DIP for DashPay shielded payment addresses",
      additions: 524,
      deletions: 1,
      updated: 366000,
      ci: "passing",
      reviewDecision: "reviewRequired",
      humans: [{ login: "PastaPastaPasta", avatarUrl: null, state: "commented" }],
      bot: { state: "none" },
    }),
  ];

  const evoTool: PullRequest[] = [
    pr(now, {
      repo: "dashpay/dash-evo-tool",
      number: 1161,
      title: "fix(ci): isolate release signing and constrain agent write paths",
      author: "lklimek",
      isFollowed: true,
      additions: 412,
      deletions: 88,
      updated: 909,
      ci: "pending",
      nextStep: step("waiting-build", "Waiting for CI", "ci", "Waiting for the required checks to finish."),
      bot: {
        state: "queued",
        position: 3,
        startsAt: at(now, 891),
        finishesAt: at(now, 891 + 1924),
        reason: "debounce (more pushes may follow)",
        priority: false,
      },
    }),
    pr(now, {
      repo: "dashpay/dash-evo-tool",
      number: 1213,
      lastReview: 2 * 86400,
      // Queued again. Its last round: one blocker from the bot, and two
      // suggestions, both CodeRabbit's.
      findings: counts([1, 1, 0], [2, 1, 0, 2]),
      merge: blocked(CHECKS_RUNNING),
      title: "refactor(tokens): token send wording and shared UI blocks",
      author: "Claudius-Maginificent",
      isFollowed: true,
      additions: 960,
      deletions: 702,
      updated: 909,
      ci: "pending",
      nextStep: step("waiting-bots", "Waiting for bots", "bots", "Waiting for bot review - thepastaclaw not yet."),
      bot: {
        state: "queued",
        position: 2,
        startsAt: at(now, 891),
        finishesAt: at(now, 891 + 1924),
        reason: "debounce (more pushes may follow)",
        priority: false,
      },
    }),
    pr(now, {
      repo: "dashpay/dash-evo-tool",
      number: 1219,
      lastReview: 2 * 3600 + 600,
      // CodeRabbit in the blockers only.
      findings: counts([2, 0, 1, 1], [3, 3, 0], [2, 0, 0, 2]),
      unresolvedThreads: 1,
      merge: unresolved(1),
      title: "feat(wallet): show pending asset lock proofs and let the user retry them",
      author: "lklimek",
      isFollowed: true,
      additions: 734,
      deletions: 210,
      updated: 2 * 3600 + 300,
      ci: "passing",
      bot: { state: "changesRequested", at: at(now, -(2 * 3600 + 600)) },
      nextStep: step(
        "waiting-self-review",
        "Author's move",
        "author",
        "thepastaclaw requested changes on this head; dismiss the review or push a fix.",
      ),
    }),
    pr(now, {
      repo: "dashpay/dash-evo-tool",
      number: 1207,
      lastReview: 26 * 3600,
      // CodeRabbit in both, the blockers all settled. Back to draft since,
      // so the bot skips the new head.
      findings: counts([1, 1, 0, 1], [5, 2, 1, 3], [4, 4, 0, 4]),
      isDraft: true,
      unresolvedThreads: 2,
      merge: blocked("This pull request is a draft."),
      title: "fix(contracts): keep the contract chooser open after a failed fetch",
      author: "pauldelucia",
      isFollowed: true,
      additions: 188,
      deletions: 61,
      updated: 25 * 3600,
      ci: "passing",
      bot: { state: "draft" },
      nextStep: step("draft", "Draft", "author", "The pull request is a draft; mark it ready for review to continue."),
    }),
  ];

  const dash: PullRequest[] = [
    pr(now, {
      repo: "dashpay/dash",
      number: 7887,
      // Opened in the last 3 days, for the created recently switch.
      created: 86400,
      milestone: "24.1",
      lastReview: 5 * 3600,
      // Queued after a push: some suggestion threads resolved since, not re-checked yet.
      findings: counts([2, 1, 0], [6, 2, 1, 0, 3], [3, 2, 0]),
      unresolvedThreads: 5,
      merge: blocked(NO_PERMISSION),
      title: "feat: isolate Evo state per chainstate while an AssumeUTXO snapshot is active",
      author: "PastaPastaPasta",
      isFollowed: true,
      additions: 2210,
      deletions: 640,
      updated: 247,
      ci: "pending",
      humans: [
        { login: "UdjinM6", avatarUrl: null, state: "changesRequested" },
        { login: "knst", avatarUrl: null, state: "commented" },
      ],
      bot: {
        state: "queued",
        position: 6,
        startsAt: at(now, 1553),
        finishesAt: at(now, 1553 + 1924),
        reason: "debounce (more pushes may follow)",
        priority: false,
      },
      rounds: 40,
      nextStep: step(
        "ready-for-human",
        "Ready for human review",
        "reviewers",
        "Ready for review - src/evo: UdjinM6 or knst | src/llmq: QuantumExplorer or PastaPastaPasta",
      ),
      needsMeReason: "Your review is wanted",
    }),
  ];

  return {
    generatedAt: at(now, 0),
    viewer: { login: VIEWER, avatarUrl: null },
    signInRequired: false,
    t3Available: true,
    projects: [
      { repo: "dashpay/platform", openCount: 137, prs: platform, error: null },
      { repo: "dashpay/dips", openCount: 12, prs: dips, error: null },
      { repo: "dashpay/dash-evo-tool", openCount: 41, prs: evoTool, error: null },
      { repo: "dashpay/dash", openCount: 96, prs: dash, error: null },
    ],
    github: { ok: true, lastSuccessAt: at(now, -12), error: null },
    feed: {
      ok: true,
      lastSuccessAt: at(now, -12),
      error: null,
      dataAsOf: at(now, -40),
      running: 4,
      queued: 10,
      medianReviewMinutes: 19.4,
      degraded: false,
    },
  };
}

/** Open PRs by other people, for the follow picker. */
export function mockOpenPrs(repo: string, nowMs: number = Date.now()): PrSummary[] {
  const snap = mockSnapshot(nowMs);
  const known: PrSummary[] = (snap.projects.find((p) => p.repo === repo)?.prs ?? []).map((p) => ({
    id: p.id,
    repo: p.repo,
    number: p.number,
    title: p.title,
    url: p.url,
    author: p.author,
    isDraft: p.isDraft,
    isMine: p.isMine,
    isFollowed: p.isFollowed,
    updatedAt: p.updatedAt,
  }));
  const others: PrSummary[] = [
    ["shumkov", "feat(drive): batch grovedb operations for identity updates"],
    ["lklimek", "fix(dapi): retry transient gRPC failures with backoff"],
    ["pauldelucia", "chore(release): update changelog and bump version"],
    ["PastaPastaPasta", "refactor: split validation into stateless and stateful stages"],
  ].map(([author, title], i) => ({
    id: `${repo}#${9000 + i}`,
    repo,
    number: 9000 + i,
    title,
    url: `https://github.com/${repo}/pull/${9000 + i}`,
    author,
    isDraft: i === 2,
    isMine: false,
    isFollowed: false,
    updatedAt: at(nowMs, -3600 * (i + 1)),
  }));
  return [...known, ...others];
}

/**
 * Made-up people the mock knows, for tracking contributors, in GitHub's
 * casing. None of their PRs is in `mockSnapshot`; tracking someone adds theirs.
 */
export const MOCK_PEOPLE = ["OrbitWren", "kestrel-dev", "tidepools", "LumenAsh", "northwind-automation-collective"];

/** The open PRs of one of `MOCK_PEOPLE` (any casing) in the mock projects; [] for anyone else. */
export function mockContributorPrs(login: string, nowMs: number = Date.now()): PullRequest[] {
  const now = nowMs;
  const author = MOCK_PEOPLE.find((p) => p.toLowerCase() === login.toLowerCase());
  if (!author) return [];
  const seeds: PrSeed[] = contributorSeeds(now).filter((s) => s.author === author);
  return seeds.map((s) => pr(now, s));
}

/** Everyone in `MOCK_PEOPLE` with their open PR count, most first. */
export function mockPeopleCounts(nowMs: number = Date.now()): { login: string; openPrs: number }[] {
  return MOCK_PEOPLE.map((login) => ({ login, openPrs: mockContributorPrs(login, nowMs).length })).sort(
    (a, b) => b.openPrs - a.openPrs,
  );
}

function contributorSeeds(now: number): PrSeed[] {
  const waiting = step(
    "waiting-bots",
    "Waiting for bots",
    "bots",
    "Waiting for bot review - thepastaclaw not yet.",
  );
  const human = step("ready-for-human", "Ready for human review", "reviewers", "Ready for review.");
  return [
    {
      repo: "dashpay/platform",
      number: 6041,
      author: "OrbitWren",
      milestone: "v5.1.0",
      headBranch: "feat/sdk-token-pricing",
      title: "feat(sdk): read token pricing schedules in the Rust SDK",
      additions: 842,
      deletions: 120,
      updated: 1800,
      ci: "passing",
      reviewDecision: "reviewRequired",
      lastReview: 1500,
      rounds: 2,
      findings: counts([1, 0, 0], [2, 1, 0]),
      unresolvedThreads: 1,
      merge: unresolved(1),
      bot: { state: "changesRequested", at: at(now, -1500) },
      nextStep: step("waiting-author", "Author's move", "author", "thepastaclaw requested changes on this head."),
    },
    {
      repo: "dashpay/platform",
      number: 6044,
      // Opened in the last 3 days, for the created recently switch.
      created: 5 * 3600,
      author: "OrbitWren",
      milestone: "v5.2.0",
      title: "fix(drive): count withdrawal documents per status in one query",
      additions: 96,
      deletions: 41,
      updated: 300,
      ci: "pending",
      merge: blocked(CHECKS_RUNNING),
      bot: {
        state: "queued",
        position: 2,
        startsAt: at(now, 640),
        finishesAt: at(now, 640 + 1500),
        reason: null,
        priority: false,
      },
      nextStep: waiting,
    },
    {
      repo: "dashpay/dash-evo-tool",
      number: 1226,
      author: "OrbitWren",
      isDraft: true,
      title: "feat(ui): show token balances on the wallet screen",
      additions: 410,
      deletions: 12,
      updated: 4 * 3600,
      ci: "none",
      merge: blocked("This pull request is a draft."),
      bot: { state: "draft" },
      nextStep: step("draft", "Draft", "author", "The pull request is a draft; mark it ready for review to continue."),
    },
    {
      repo: "dashpay/platform",
      number: 6047,
      author: "kestrel-dev",
      milestone: "v5.1.0",
      title: "fix(dapi): keep the subscription open across a masternode list update",
      additions: 233,
      deletions: 75,
      updated: 2 * 3600,
      ci: "failing",
      lastReview: 2 * 3600,
      rounds: 1,
      merge: blocked("Required checks are failing."),
      bot: { state: "commented", at: at(now, -2 * 3600) },
      findings: counts([0, 0, 0], [1, 0, 0]),
      nextStep: step("waiting-build", "CI failing", "author", "A required check failed."),
    },
    {
      repo: "dashpay/dash",
      number: 7892,
      author: "kestrel-dev",
      milestone: "24.1",
      title: "test: cover the ChainLock signing path when the quorum rotates",
      additions: 318,
      deletions: 4,
      updated: 26 * 3600,
      ci: "passing",
      reviewDecision: "approved",
      humans: [{ login: "PastaPastaPasta", avatarUrl: null, state: "approved" }],
      bot: { state: "approved", at: at(now, -26 * 3600) },
      lastReview: 26 * 3600,
      merge: blocked(NO_PERMISSION),
      nextStep: human,
    },
    {
      repo: "dashpay/platform",
      number: 6049,
      author: "tidepools",
      milestone: "v5.2.0",
      title: "refactor(dpp): move document type validation into its own module",
      additions: 1204,
      deletions: 1150,
      updated: 900,
      ci: "passing",
      lastReview: 120,
      bot: {
        state: "reviewing",
        phase: "phase2",
        phaseLabel: "Phase 2",
        progress: 0.62,
        finishesAt: at(now, 520),
        startedAt: at(now, -900),
        overdue: false,
        tier: "normal",
        priority: false,
        waitedSeconds: 140,
        steps: [
          { key: "triage", label: "Triage", status: "done", detail: null },
          { key: "phase1", label: "Phase 1", status: "done", detail: null },
          { key: "verify1", label: "Verify", status: "done", detail: null },
          { key: "gate", label: "Gate", status: "done", detail: null },
          { key: "phase2", label: "Phase 2", status: "running", detail: "1 of 2 lanes" },
          { key: "verify2", label: "Verify 2", status: "pending", detail: null },
          { key: "publish", label: "Publish", status: "pending", detail: null },
        ],
        findings: null,
      },
      merge: blocked(CHECKS_RUNNING),
      nextStep: waiting,
    },
    {
      repo: "dashpay/dips",
      number: 197,
      author: "tidepools",
      title: "docs: clarify how identities pay for contested names",
      additions: 88,
      deletions: 23,
      updated: 3 * 86400,
      ci: "passing",
      reviewDecision: "reviewRequired",
      bot: { state: "none" },
      merge: blocked(BRANCH_RULES),
    },
    {
      repo: "dashpay/dash-evo-tool",
      number: 1229,
      author: "LumenAsh",
      title: "fix(network): reconnect to DAPI after the laptop wakes from sleep",
      additions: 61,
      deletions: 18,
      updated: 5400,
      ci: "passing",
      lastReview: 3000,
      rounds: 1,
      bot: { state: "approved", at: at(now, -3000) },
      merge: READY,
      nextStep: human,
    },
    {
      repo: "dashpay/platform",
      number: 6051,
      author: "northwind-automation-collective",
      title: "chore(deps): bump tokio from 1.44 to 1.45",
      additions: 14,
      deletions: 14,
      updated: 7 * 3600,
      ci: "passing",
      bot: { state: "none" },
      merge: blocked(BRANCH_RULES),
    },
  ];
}
