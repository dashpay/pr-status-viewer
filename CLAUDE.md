# PR Status

A desktop app (macOS and Linux) that shows the pull requests you care about:
your own open PRs in the projects you track, plus PRs you follow. For each it
shows the review bot's verdict, the time until its review finishes, human
reviewers, and CI.

Stack: Tauri 2. Rust backend in `src-tauri/`, React 19 + TypeScript UI in `src/`.

## How data flows

```
GitHub GraphQL ──> github::GitHubClient ──┐
                                          ├──> merge::build_snapshot ──> model::Snapshot ──> UI
review feed JSON ─> feed::FeedClient ─────┘         (pure)              (event "snapshot")
```

- `src-tauri/src/model.rs` and `src/types.ts` are the same contract in two
  languages; change them together. JSON is camelCase; timestamps are RFC 3339 UTC strings.
- `src-tauri/src/github/types.rs` is the hand-off from the GitHub client to `merge`.
- `src/api.ts` is the only file that talks to the backend. In a plain browser
  and in tests it is a mock (`src/mock/`), so the UI runs without the backend.
- `src/state/store.ts` holds app state; components read it with `useApp(selector)`.
- `src/lib/select.ts` decides which PRs belong to Mine / Following / Needs me.
- `src/styles/tokens.css` and `base.css` hold every colour and the shared
  primitives (`.pill`, `.ci`, `.meter`, `.kbd`, `.seg`, `.btn`, `.input`).

The engine in `sync/` owns all state and talks to the outside world through
the traits in `sync/source.rs`, so its tests run on fakes and a paused clock.
It publishes nothing until the first GitHub attempt has finished, and it never
saves over a config file it could not read.

The review bot is the GitHub user `thepastaclaw`. Its verdict is the check run
named `PastaClaw review` on each PR head commit; live progress and queue
position come from its public feed (`feed::FEED_URL`), which has no stability
promise and must be parsed tolerantly. A finished check run newer than the
feed beats the feed; a feed older than 30 minutes is not used for live state.

The commit status named `PR Hygiene` is a workflow gate, not CI. It is left
out of the CI state and shown as the PR's "next step" (`merge/next_step.rs`).
GitHub App reviewers (GraphQL type `Bot`) are not listed as human reviewers.

Squash merge: `merge/readiness.rs` is the one place that decides whether a PR
can be merged (every review thread resolved, and GitHub would allow it). It
also owns `has_conflicts` (mergeable CONFLICTING or state DIRTY), which feeds
both its "Merge conflicts" rule and `PullRequest::has_conflicts`, the row's
conflict icon; the flag is set whatever rule comes first. The
`squash_merge` command never trusts the snapshot or the UI: it fetches a fresh
check, refuses if the head moved or the PR is not ready, and only then calls
the mutation. Never run that mutation against GitHub from tests, examples or
scripts; it is exercised on fakes only.

Self-review: the gate's controller (dashpay/stale_prs_are_bad) accepts a
comment that is exactly `/self-reviewed <40-hex head sha>` from the PR's author
or an assignee. `merge/self_review.rs` is the one place that decides whether it
is offered (`SelfReviewState`) and builds that comment. `post_self_review`
works like `squash_merge`: a fresh check, refuse if the head moved or the
viewer does not hold the PR, only then the `addComment` mutation. It is the
user's own statement: never send that mutation to GitHub from tests, examples,
scripts or by hand; it is exercised on fakes only.

Findings cover only the LAST review round of each of two reviewers, the
review bot and CodeRabbit (`github/rounds.rs` picks the rounds:
for the bot, the threads its last full review opened plus the earlier ones
that review links to; for CodeRabbit, the threads its last review opened).
Severity and status are parsed from each thread's first comment
(`github/finding.rs`, `github/coderabbit.rs`: Critical and Major count as
blockers, Minor as suggestions) and counted in `merge/findings.rs`. A
finding that still stands in a thread resolved on GitHub is "not agreed"
only when its reviewer's last round was on the PR's current head; when that
round was on an older commit it is "awaiting" (the author pushed a fix and
resolved the thread, and the reviewer has not re-checked it yet). The row's
"round N" (`PullRequest::rounds`) is how many reviews the bot has posted on
the PR: its reviews whose body carries the round marker (`rounds::is_post`),
not its thread replies or "CI failed after approval" follow-ups. It is
counted in the same follow-up: from the window of latest bot reviews when
that holds them all, else counted on from the cached count, else (first read
or a gap) by reading all the bot's reviews once, page by page. The feed's
`rounds` section is not used. Comment bodies are
expensive to query, so the page queries stay cheap (thread ids and resolved
flags only) and a follow-up query fetches bodies only for PRs that changed or
whose cached copy is five minutes old (`github/finding_cache.rs`). Watch the
rate-limit cost when touching these queries: `examples/findings_cost.rs`.

Traits (`github/traits.rs`): a workflow in dashpay/platform comments on a PR
that changes the described GroveDB structure or a system contract's schemas.
Only comments by the Bot `github-actions` that start with the workflow's
marker count, and a viewer link is kept only when it is on
`dashpay.github.io` under that trait's viewer path, because the app opens it.
Comments are read in the same cached follow-up as findings.

Tracked contributors (`Config::contributors`): the snapshot also carries
their open PRs in the tracked projects. They ride on the viewer's own search
(`github/search.rs` builds it: repeated `author:` qualifiers are ORed, split
across searches when the string gets long), so tracking adds no query per
person. Such a PR has `is_mine` and `is_followed` false; the UI shows it only
in that person's view (`View` kind `contributor`, matched by author) and the
project lists keep to `isListed` PRs (`lib/select.ts`). If the capped combined
search is truncated, the viewer's PRs are fetched again on their own.

Stacked PRs: `merge/stack.rs` sets `PullRequest::stacked_on`, the PR in the
same project whose head branch (not from a fork, per `isCrossRepository`) is
this PR's base branch, among the PRs in the snapshot. No parent when the head
branch is ambiguous, when the branch is the base of more than three PRs (a
shared branch, such as a release PR's head), or in a cycle. The UI
(`lib/stack.ts`, applied in `lib/group.ts`) puts each stacked PR right after
its parent within the same rendered list, with a ↳ and one indent step per
level; a parent outside that list leaves the child at its own place.

List filters, all in the UI and all remembered between runs: a drafts switch,
a "created recently" switch (when on, only PRs created in the last 72 hours)
and milestones (`lib/milestones.ts`), all as chips above the list, and
per-project areas (`lib/areas.ts`, checkboxes in the sidebar; an area is the
scope in the PR title, as in `fix(sdk): ...`). `lib/filters.ts` combines them
with the text filter, checking drafts, then age, then milestone, then area,
and counts each hidden PR once. Sidebar counts are totals and ignore the filters.

`t3/` finds the T3 Code threads tied to a PR through T3 Code's own CLI,
`t3 thread list --json --project <dir>` (`t3/cli.rs`, located through
`PR_STATUS_T3_CLI`, `$T3CODE_HOME/bin/t3` or `~/.t3/bin/t3`, never PATH),
run for each T3 project whose git origin is a tracked or followed repo, in
parallel, with a timeout. The CLI has no project list, so the project
directories still come from one read-only query of T3 Code's SQLite database
(`t3/projects.rs`, cached for five minutes; `run/` uses it too). A thread is
working, waiting (its turn ended while it waits on something it started,
usually a watch on the PR) or idle. The CLI does not list delegated child
threads. The format is not ours: any surprise means "unavailable", never an
error shown to the user. The only `t3` command the app may run is
`thread list`, and nothing may ever write to T3 Code's files.

`run/` builds a pull request's code and opens the project's iOS or Android
example app on a simulator or emulator. It is separate from the sync engine:
the commands look the PR up in the engine's snapshot and hand the runner plain
facts. Recipes are data (`run/recipes.rs`; dashpay/platform is the only entry).
The code comes either from one of the user's worktrees on the PR's branch,
built in place with no git writes at all, or from the PR head in the app's own
checkout (a detached worktree of the user's clone under the app cache dir,
one per project). The only git writes ever made in the user's clone are
`worktree add` for that path. A job runs one step at a time, each command in
its own process group so cancel stops the whole tree; `run_start` re-validates
everything itself. The runner talks to the outside through the traits in
`run/host.rs` and `run/source.rs`, so its tests use fakes; real builds are
only ever started by the user or by `examples/run_smoke.rs`, and never on a
device that holds the user's own data. The step a cancelled job was in goes
back to `pending` with its start time kept; the UI shows that as "Stopped".

## Commands

```
npm run typecheck        # tsc
npm test                 # vitest, UI unit tests
npm run dev              # UI alone in a browser on :1420, mock data
npm run tauri dev        # the real app
cd src-tauri && cargo test
cd src-tauri && cargo clippy --all-targets -- -D warnings
cd src-tauri && cargo fmt
cd src-tauri && cargo run --example snapshot_dump -- owner/name   # one real sync, printed as JSON
cd src-tauri && cargo run --example run_smoke -- 'owner/name#N' --list   # what could be run for a PR (read-only)
```

In a browser, `?snapshot=<url>` loads a captured snapshot instead of the mock
data (see the README).

## Conventions

- Rust: no `unwrap`/`expect` outside tests; return `AppResult<T>`. Network
  code never runs in tests; parse functions are pure and tested on fixtures in
  `src-tauri/tests/fixtures/`. Never log a token.
- UI: plain CSS using the tokens, one `.css` file beside the component that
  owns it. No colour, radius or font literals outside `tokens.css`. Status is
  never shown by colour alone: every state has a text label.
- User-facing text is sentence case, plain, and names the thing ("Add
  project", not "Submit").
- Tests sit beside the code: `foo.test.ts(x)` in the UI, `#[cfg(test)]` in Rust.
