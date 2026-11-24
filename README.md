# PR Status

PR Status is a desktop app for macOS and Linux that keeps an eye on the GitHub
pull requests you care about. You tell it which projects (GitHub repositories)
you track, and it lists your own open pull requests in those projects plus any
other pull requests you choose to follow. It is built around a review bot, the
GitHub user `thepastaclaw`, that reviews pull requests in those projects.

For each pull request it shows:

- when the review bot's last review came in, or that a review is running or
  queued and how long until it finishes
- how the findings of the last review round stand, as two bubbles: red for
  blockers, yellow for suggestions. Each shows thumbs-down (closed without
  the reviewer's agreement: disputed, deferred, or resolved although the
  reviewer has reviewed the current code and the finding still stands), an
  hourglass (resolved on GitHub, but the reviewer has not reviewed the latest
  commit yet, so the fix is awaiting re-review), thumbs-up (settled with its
  agreement) and the total, for example "0 / 3", "hourglass 5 / 5" or
  "thumbs-up 2 / 2". The counts take the last round of the
  review bot and the last round of CodeRabbit; a rabbit in a bubble means
  its total includes CodeRabbit findings (Critical and Major count as
  blockers, Minor as suggestions). Findings a reviewer lists without opening
  a thread are not counted, and counts can lag by up to five minutes.
- the bot's verdict, in the pull request's detail
- the human reviewers and where their reviews stand
- the state of CI on the latest commit
- the next step and whose move it is, in projects that use the "PR Hygiene"
  status (for example "Your move" or "Ready for human review")
- its traits, as icons you can click: a tree when it changes the described
  GroveDB structure, a page when it adds or changes a system contract's
  schemas. Clicking opens the structure viewer or the contract viewer in your
  browser. These come from a workflow's comments in dashpay/platform; other
  projects show none.
- how many review comment threads are still unresolved
- whether a T3 Code thread is tied to it, and whether an agent is working in
  that thread right now or it is waiting (for example watching the pull
  request)
- merge conflicts with the base branch, as a red merge icon at the end of the
  title line (after the T3 Code tag); hover it for "Merge conflicts with
  <base>". GitHub works this out lazily, so a fresh push can take a sync or
  two to show it.
- stacked pull requests: a pull request whose base branch is another listed
  pull request's branch sits right under that one, marked with ↳ and indented
  one step per level (hover the arrow for "Stacked on #N"). A branch that more
  than three pull requests target counts as shared, not a stack, and a
  stacked pull request whose parent is filtered out or in another group is
  listed on its own.

The pull requests are grouped into three views: Mine (your own), Following
(the ones you follow) and Needs me (the ones waiting on you to act), and
each project has its own list too.

Above the list, "Group" lists the pull requests by project or by status, and
"Sort" puts the ones whose review finishes first, or the most recently
updated, on top. When grouped by project, tick "By subcrate" to split each
project into one list per subcrate: the scope in the title, such as `sdk` in
`fix(sdk): ...`, with "Other" for titles without one. A pull request whose
title names several goes under the first one you have not switched off in
the sidebar.

## Requirements

- macOS 11 or later, or a recent 64-bit Linux desktop. Development so far has
  been on macOS; the Linux build is exercised only by the CI workflow.
- A GitHub account and a way for the app to get a token (see
  [Signing in to GitHub](#signing-in-to-github)).

To build it from source you also need:

- Node.js 22 and npm
- Rust, stable toolchain (install with [rustup](https://rustup.rs))
- On macOS: the Xcode command line tools (`xcode-select --install`)
- On Debian or Ubuntu: the system libraries Tauri needs. This installs them:

  ```sh
  scripts/install-linux-deps.sh
  ```

  It runs `apt-get` with `sudo` for `build-essential`, `curl`, `wget`,
  `file`, `libwebkit2gtk-4.1-dev`, `libxdo-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf` and
  `xdg-utils`. For other distributions, see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and leave
  out OpenSSL, which this app does not need.

## Run it from source

```sh
git clone https://github.com/dashpay/pr-status-viewer.git
cd pr-status-viewer
npm ci
npm run tauri dev
```

`npm run tauri dev` starts the UI's development server on port 1420, builds
the Rust side and opens the app window. The first build takes a few minutes;
later ones are quick. Changes to the UI reload in place; changes to the Rust
code rebuild and restart the app.

## Build a bundle

```sh
npm ci
npm run tauri build
```

This builds a release version and packages it. The bundles land in
`src-tauri/target/release/bundle/`:

| Platform | Bundle                          | Folder        |
| -------- | ------------------------------- | ------------- |
| macOS    | `PR Status.app`                 | `macos/`      |
| macOS    | Disk image (`.dmg`)             | `dmg/`        |
| Linux    | Debian package (`.deb`)         | `deb/`        |
| Linux    | AppImage (`.AppImage`)          | `appimage/`   |

To build one macOS app that runs on both Apple silicon and Intel Macs:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin
```

The bundles then land in
`src-tauri/target/universal-apple-darwin/release/bundle/`.

The bundles are not code signed yet. On macOS, the first time you open the
app, Control-click it in Finder and choose Open, then confirm.

## Signing in to GitHub

PR Status has no sign-in screen. When it starts, it looks for a GitHub token
in this order and uses the first one it finds:

1. the `GH_TOKEN` environment variable
2. the `GITHUB_TOKEN` environment variable
3. the [GitHub CLI](https://cli.github.com), by running `gh auth token`

If you already use the GitHub CLI and have run `gh auth login`, there is
nothing to set up. The app keeps the token in memory only: it never writes it
to disk and never logs it.

Environment variables reach the app only when you start it from a shell (for
example with `npm run tauri dev`, or by running the AppImage from a
terminal). An app opened from the Dock, Finder or a desktop launcher does not
see variables set in your shell profile, so in that case rely on the GitHub
CLI.

The token only needs to read. It needs read access to the repositories you
track:

- A fine-grained personal access token: give it access to those repositories
  with read-only permission for Contents, Pull requests, Checks and Commit
  statuses (Metadata is included automatically).
- A classic personal access token: the `repo` scope if any of the
  repositories are private. For public repositories only, a token with no
  scopes is enough.
- The GitHub CLI's own token already has the access it needs.

## Where the data comes from

- **GitHub's GraphQL API** gives the pull requests, their reviewers, the
  check runs and commit statuses that make up CI, and the review bot's
  verdict. The verdict is the check run named `PastaClaw review` on the
  pull request's latest commit.
- **The review bot's public status feed** at
  <https://thepastaclaw.github.io/review-system/data/status.json> gives the
  bot's live progress: which reviews are running, the queue and how long
  reviews usually take. This is an unofficial file with no stability promise;
  its format can change without notice. When the app cannot read it, it
  still shows what GitHub reports.

The finish time for a queued review is this app's own estimate, worked out
from the bot's median review time. The bot does not publish one, so treat it
as a guide.

## Squash and merge

Each pull request's detail has a "Squash and merge" button. It is enabled only
when every review comment thread on the pull request is resolved and GitHub
itself would allow you to squash-merge it. Otherwise it is disabled and says
why: unresolved comments, merge conflicts, a branch rule that is not met, a
draft, and so on.

- Clicking it opens a confirmation that asks GitHub again and shows the
  branch it merges into and the squash commit title. Nothing merges until you
  confirm there.
- When you confirm, the app checks once more and refuses if a thread has been
  reopened or new commits have been pushed since the confirmation opened.
- The app never bypasses branch protection, even if your account could.
- "Comments" means review threads. Top-level comments on a pull request
  cannot be resolved on GitHub, so they are not counted.
- A pull request with only optional checks failing can be merged, as on
  GitHub; the confirmation warns you.
- Merging cannot be undone from the app.

## Posting the self-review

Projects that use the "PR Hygiene" gate ask whoever holds a pull request (its
author, or anyone it is assigned to) to state that they have read the final
diff themselves, by commenting `/self-reviewed`. You can post that from the
app instead of going to GitHub.

- A pull request you hold has a "Post /self-reviewed" button in its detail
  (and in the command palette). It stands out when the gate says that is the
  next step, and the row then reads "Your move: self-review". It is plain when
  the gate is waiting on something else first, and absent on drafts, on pull
  requests you do not hold, and once the gate is past that step.
- Clicking it opens a confirmation that asks GitHub again and shows the
  commit, what the gate says now, a link to the diff, and the exact comment.
  Nothing is posted until you confirm there.
- The comment is always `/self-reviewed <full head commit>`, which the gate
  ties to that commit: a later push that changes the diff takes it back.
- When you confirm, the app checks once more and refuses if new commits have
  been pushed since the confirmation opened, so you never attest to code you
  have not seen.
- The gate re-evaluates a minute or two after the comment. Until it does, the
  pull request reads "Self-review posted". Posting again on the same commit
  is allowed; the gate asks for a renewed one after an objection is answered.
- The comment is posted as you and stays on the pull request; the app does
  not delete or edit it (the gate ignores edited comments).

GitHub cuts the gate's description at 140 characters, so on a pull request
with a long list of things to address the request for the self-review can be
cut off; the button is then plain rather than highlighted.

## T3 Code

If T3 Code is installed and running, the app shows which of its threads are
tied to each pull request (the pull requests T3 Code links to the thread).
A row is marked "T3 working" (in blue) while an agent is working in such a
thread, and "T3 waiting" when the thread's turn has ended but it waits on
something it started, usually a watch on the pull request that wakes it when
CI, reviews or the merge state change. While T3 is working on one of your
pull requests, its row does not say "Your move": the agent has it. The
detail lists each thread and says when T3
Code is watching the pull request for it. Settled threads are left out
unless they are working or waiting.

The app asks T3 Code's own command, `t3 thread list --json`, about every 30
seconds, only for the T3 projects whose clone points at a project you track
or a pull request you follow. It never runs any other `t3` command. The
list of T3 projects comes from T3 Code's local database,
`~/.t3/userdata/statev2.sqlite`, read-only, at most every five minutes.
When T3 Code is not running, or its answer cannot be read, the T3 Code
information is simply not shown. Threads that T3 Code starts for delegated
tasks are not listed by its command, so a pull request linked only from one
of those is not shown. Set `PR_STATUS_T3_CLI` to use a different `t3`
command and `PR_STATUS_T3_DB` to read a different database.

## Running the example apps

For a project that has example apps, an open pull request's detail has "Run on
iOS" and "Run on Android" (also in the command palette). Today that means
dashpay/platform, with `SwiftExampleApp` and `KotlinExampleApp`. The app
builds the code, starts the simulator or emulator if needed, installs the
example app and opens it, and shows the steps and the build log in a panel
under the list. One run at a time; "Cancel" stops the whole build.

You choose two things, and the app remembers them:

- **Code.** "Your worktree" is a worktree of your clone that is on the pull
  request's branch, built in place exactly as it is; the dialog says whether
  it is at the pull request's head and whether it has uncommitted changes. It
  is the default when you have one. "Pull request head" is a clean copy the
  app keeps for itself, one per project, reused between pull requests so later
  builds are incremental.
- **Device.** An iOS simulator, an Android emulator, or an Android device
  connected over adb. Physical iPhones are not offered. Running replaces the
  example app on that device and keeps its data.

The first build of each platform compiles the Rust SDK from scratch, which
took 15 to 20 minutes on the machine this was developed on. After that, a run
of the same or a nearby commit takes a minute or two.

The app needs a local clone of the project. It finds one among T3 Code's
projects; otherwise set it with "Local clone…" in the project's menu in the
sidebar.

What must be installed:

- git with access to the clone's `origin` without a prompt (for an SSH remote,
  the key loaded in the agent), and rustup.
- iOS (macOS only): Xcode, a simulator, and the `aarch64-apple-ios-sim` Rust
  target.
- Android: the Android SDK with platform-tools, the emulator and NDK r28 or
  newer (found through `ANDROID_HOME`, else `~/Library/Android/sdk` or
  `~/Android/Sdk`), JDK 17 (through `JAVA_HOME`, `/usr/libexec/java_home`,
  Homebrew's `openjdk@17` or Android Studio), `cargo-ndk`, and an emulator or
  a device with USB debugging on.

Where things are kept (macOS paths; on Linux under `~/.cache/org.dash.prstatus`
and `~/.config/org.dash.prstatus`):

| What | Where |
| --- | --- |
| The app's checkout, with its build cache | `~/Library/Caches/org.dash.prstatus/checkouts/<owner>-<name>` |
| Xcode build data | `~/Library/Caches/org.dash.prstatus/xcode/` |
| Logs of the last ten runs | `~/Library/Caches/org.dash.prstatus/logs/` |
| A local clone you set by hand | `~/Library/Application Support/org.dash.prstatus/run.json` |

The app's checkout is a detached git worktree of your clone, so it appears in
`git worktree list`. It grows to several GB; deleting the folder is safe, and
the next run makes it again (cold). Anything edited inside it is discarded by
the next run.

Limits: a worktree is matched to a pull request by branch name, so a fork's
pull request from a branch named like one of yours is offered that worktree
(marked as not at the pull request's head). A simulator or emulator the app
started keeps running after the run. This has been run for real on macOS with
an iOS simulator and an Android emulator; a physical Android device and Linux
are untested.

## Filtering the list

- **Drafts:** draft pull requests carry a "Draft" tag at the start of their
  title and are drawn quieter. The "Drafts" chip above the list shows or
  hides all of them.
- **Created recently:** tick the "Created recently" chip, beside "Drafts", to
  list only pull requests opened in the last 3 days; its number says how many
  there are. Unticked, it hides nothing.
- **Milestones:** when the pull requests in a view span more than one
  milestone, a row of chips appears above the list, one per milestone plus "No
  milestone". Click a chip to hide or show its pull requests; Alt-click to
  show only that one.
- **Areas:** in the sidebar, the arrow beside a project opens a checkbox per
  area. An area is the scope in a pull request's title, such as `sdk` in
  `fix(sdk): ...`; titles without one are under "Other". Unticking an area
  hides its pull requests in every view.
- **Text:** the filter box matches title, number, project and author.

A pull request is listed only if it passes all of them. The end of the list says
how many are hidden and by what, and your choices are remembered. The counts in
the sidebar are totals and ignore the filters.

## Other people's pull requests

The switcher at the top right reads "Mine". Open it and choose "Track
contributor…" to follow what someone else is doing: type their GitHub
username (or `@name`, or a link to their profile), or pick from the people who
have open pull requests in your projects. Each person you track gets a view of
their open pull requests across your projects, with the same columns, filters
and detail as your own. Switch between "Mine" and the people you track from
the same menu, or from the command palette; "Stop tracking" is in that menu
while the person's view is showing.

- Your own lists do not change: Mine, Following, the project lists and the
  sidebar counts stay yours. A tracked person's pull request appears in Needs
  me only when it is waiting on you, for example when they asked for your
  review.
- You can track up to 20 people. Organisations, GitHub Apps and the review bot
  cannot be tracked.
- At most 150 pull requests are listed per project, most recently updated
  first; your own are never left out to make room.
- Tracking someone with many open pull requests makes each sync cost more of
  your GitHub API allowance. With the busiest contributor in dashpay/platform
  tracked, a sync of that project used about twice the points; still a small
  part of the hourly limit.

## Projects and followed pull requests

- **Add a project:** choose Add project in the sidebar and enter
  `owner/name` or any github.com link inside the repository. The app checks
  the repository exists before adding it.
- **Follow a pull request:** open a project's menu in the sidebar to browse
  its open pull requests and follow the ones you want, or follow one directly
  by entering `owner/name#123` or its pull request link. Followed pull
  requests appear under Following.
- **Remove a project:** use the project's menu in the sidebar. Removing a
  project also stops following its pull requests.

The app saves your projects and followed pull requests in `config.json` in
its configuration folder: `~/Library/Application Support/org.dash.prstatus/`
on macOS, and `~/.config/org.dash.prstatus/` on Linux (or under
`$XDG_CONFIG_HOME` if you set it).

## Development

The app is built with [Tauri 2](https://v2.tauri.app): a Rust backend fetches
and merges the data, and a React and TypeScript UI shows it.

| Command                                                | What it does                               |
| ------------------------------------------------------ | ------------------------------------------ |
| `npm run dev`                                          | The UI alone in a browser, with mock data  |
| `npm run tauri dev`                                    | The real app                               |
| `npm run typecheck`                                    | Type-checks the UI                         |
| `npm test`                                             | UI unit tests (Vitest)                     |
| `cd src-tauri && cargo test`                           | Rust tests                                 |
| `cd src-tauri && cargo clippy --all-targets -- -D warnings` | Rust lints                            |
| `cd src-tauri && cargo fmt`                            | Formats the Rust code                      |

`npm run dev` serves the UI at <http://localhost:1420>. In a plain browser
there is no Rust backend, so the UI uses a mock backend with made-up pull
requests from `src/mock/`. This is the quickest way to work on the UI, and
the UI tests use the same mock.

To look at real data in the browser, capture a snapshot from the backend and
point the UI at it:

```sh
cd src-tauri
cargo run --example snapshot_dump -- owner/name 'owner/name#123' > ../public/live.json
cd .. && npm run dev        # then open http://localhost:1420/?snapshot=/live.json
```

`snapshot_dump` runs the real sync once without a window and prints what the
UI would receive. Arguments with a `#` are followed pull requests; the rest
are projects. Delete `public/live.json` afterwards: it holds your pull
request data.

Project layout:

```
src/                 React UI
  api.ts             the only file that talks to the backend (mock in a browser)
  components/        views, dialogs and the pull request table
  state/, lib/       app state and the rules for Mine, Following and Needs me
  styles/            colour tokens and shared styles
src-tauri/src/       Rust backend: github/ and feed/ fetch, merge.rs combines,
                     model.rs is the data the UI receives
src-tauri/tests/     test fixtures
```

`CLAUDE.md` has more detail on how the data flows and the code conventions.

### Continuous integration and releases

`.github/workflows/ci.yml` runs on every pull request and every push to
`master`. It type-checks, tests and builds the UI; checks formatting, lints
and tests the Rust code on macOS and Ubuntu; then builds the macOS and Linux
bundles and keeps them as downloadable workflow artifacts for 14 days.

To make a release, set `version` in `src-tauri/tauri.conf.json`, merge it,
then push a matching tag:

```sh
git tag v0.2.0
git push origin v0.2.0
```

`.github/workflows/release.yml` then builds a universal macOS app and disk
image and a Linux `.deb` and AppImage, and attaches them to a draft release
on GitHub. Check the draft, then publish it.

### App icon

The icon is drawn in `assets/icon.svg`. To change it, edit the SVG, export it
as a 1024 by 1024 PNG, and regenerate the icon set:

```sh
npx tauri icon path/to/icon-1024.png -o src-tauri/icons
```

Then delete the `android/` and `ios/` folders and the `Square*Logo.png`,
`StoreLogo.png` and `icon.ico` files it creates; this app does not use them.

## Licence

MIT. See [LICENSE](LICENSE).
