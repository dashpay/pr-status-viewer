//! Turns raw GitHub facts plus the review feed into the `Snapshot` the UI
//! shows. Pure: no I/O, no clock (the caller passes `now`).

mod bot;
pub mod eta;
mod findings;
mod next_step;
mod readiness;
mod self_review;
mod stack;
mod steps;
mod t3_threads;

use std::collections::HashSet;

use chrono::{DateTime, TimeDelta, Utc};

use crate::feed::FeedSnapshot;
use crate::github::types::{GhProject, GhPullRequest, GhReviewState};
use crate::model::{
    BOT_LOGIN, BotReview, CiState, Config, FeedHealth, HumanReview, HumanReviewState, NextStep,
    ProjectSnapshot, PullRequest, Snapshot, SourceHealth, Viewer, pr_id,
};
use crate::t3::T3Link;

use readiness::has_conflicts;
pub use readiness::{CLOSED_REASON, merge_check, readiness};
pub use self_review::{
    PostedSelfReview, SELF_REVIEW_CLOSED, SELF_REVIEW_DRAFT, SELF_REVIEW_NOT_HOLDING, self_review,
    self_review_check, self_review_comment,
};
pub use t3_threads::t3_threads;

/// For the GitHub client's tests, which check what its findings count as.
#[cfg(test)]
pub(crate) use findings::finding_counts;

/// A feed whose `data_as_of` is older than this is reported as unhealthy.
const FEED_STALE_AFTER_MINUTES: i64 = 10;
/// A feed older than this is not used for live review state at all.
const FEED_UNUSABLE_AFTER_MINUTES: i64 = 30;

pub struct MergeInput<'a> {
    pub now: DateTime<Utc>,
    pub viewer: Option<Viewer>,
    pub config: &'a Config,
    /// One per configured project, in config order.
    pub projects: Vec<ProjectInput>,
    pub github: SourceHealth,
    /// The latest feed we have, even if the most recent fetch failed.
    pub feed: Option<&'a FeedSnapshot>,
    pub feed_status: SourceHealth,
    /// T3 Code threads tied to pull requests; `None` when T3 Code's data
    /// could not be found.
    pub t3: Option<&'a [T3Link]>,
    /// The self-reviews the app has posted since it started.
    pub self_reviews: &'a [PostedSelfReview],
}

pub struct ProjectInput {
    pub repo: String,
    /// The latest good fetch, if any.
    pub project: Option<GhProject>,
    /// Why the most recent fetch failed, if it did.
    pub error: Option<String>,
}

pub fn build_snapshot(input: MergeInput<'_>) -> Snapshot {
    let MergeInput {
        now,
        viewer,
        config,
        projects,
        github,
        feed,
        feed_status,
        t3,
        self_reviews,
    } = input;

    // A feed that stopped updating long ago says nothing about what is
    // running now: fall back to GitHub's check runs for live state.
    let live_feed = feed.filter(|f| {
        f.data_as_of
            .is_none_or(|t| (now - t).num_minutes() < FEED_UNUSABLE_AFTER_MINUTES)
    });

    let ctx = Context {
        viewer: viewer.as_ref().map(|v| v.login.as_str()),
        followed: config.followed.iter().map(|id| id.to_lowercase()).collect(),
        feed: live_feed,
        t3: t3.unwrap_or_default(),
        self_reviews,
    };

    let projects = projects
        .into_iter()
        .map(|p| {
            let (open_count, prs) = match p.project {
                Some(project) => (
                    project.open_count,
                    project
                        .prs
                        .iter()
                        .zip(stack::stacked_on(&project.prs))
                        .map(|(pr, stacked_on)| ctx.pull_request(&p.repo, pr, stacked_on))
                        .collect(),
                ),
                None => (0, Vec::new()),
            };
            ProjectSnapshot {
                repo: p.repo,
                open_count,
                prs,
                error: p.error,
            }
        })
        .collect();

    Snapshot {
        generated_at: now,
        // The sync engine knows why GitHub failed; it sets this afterwards.
        sign_in_required: false,
        t3_available: t3.is_some(),
        projects,
        github,
        feed: feed_health(now, feed, feed_status),
        viewer,
    }
}

/// What every PR in one merge is judged against.
struct Context<'a> {
    viewer: Option<&'a str>,
    /// Lower-cased PR ids.
    followed: HashSet<String>,
    feed: Option<&'a FeedSnapshot>,
    t3: &'a [T3Link],
    self_reviews: &'a [PostedSelfReview],
}

impl Context<'_> {
    /// `repo` is the project's name as the user tracks it. It can differ
    /// from `pr.repo` (GitHub's current name) after a rename or transfer; the
    /// tracked name is what the config, the views and followed ids use.
    /// `stacked_on` is decided for the whole project first (`stack`).
    fn pull_request(&self, repo: &str, pr: &GhPullRequest, stacked_on: Option<u64>) -> PullRequest {
        let id = pr_id(repo, pr.number);
        let key = id.to_lowercase();
        let is_mine = self
            .viewer
            .is_some_and(|v| v.eq_ignore_ascii_case(&pr.author));
        let bot = bot::bot_review(pr, self.feed);
        let humans = humans(pr);
        let next_step = pr
            .hygiene
            .as_ref()
            .and_then(|h| next_step::next_step(h, is_mine));
        let needs_me_reason = needs_me(
            pr,
            &Judged {
                is_mine,
                bot: &bot,
                humans: &humans,
                next_step: next_step.as_ref(),
            },
            self.viewer,
        );
        PullRequest {
            id,
            repo: repo.to_string(),
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            author: pr.author.clone(),
            author_avatar_url: pr.author_avatar_url.clone(),
            milestone: pr.milestone.clone(),
            is_draft: pr.is_draft,
            is_mine,
            is_followed: self.followed.contains(&key),
            additions: pr.additions,
            deletions: pr.deletions,
            created_at: pr.created_at,
            updated_at: pr.updated_at,
            head_sha: pr.head_sha.clone(),
            head_branch: pr.head_branch.clone(),
            ci: pr.ci,
            review_decision: pr.review_decision,
            humans,
            bot,
            rounds: pr.rounds,
            next_step,
            last_review_at: findings::last_review_at(pr),
            finding_counts: findings::finding_counts(&pr.findings, &pr.last_rounds, &pr.head_sha),
            traits: pr.traits.clone(),
            base_branch: pr.merge.base_branch.clone(),
            stacked_on,
            unresolved_threads: pr.merge.unresolved_threads,
            merge: readiness(&pr.merge, pr.is_draft),
            has_conflicts: has_conflicts(&pr.merge),
            self_review: self_review(pr, repo, self.viewer, self.self_reviews),
            t3_threads: t3_threads(self.t3, repo, &pr.repo, pr.number),
            needs_me: needs_me_reason.is_some(),
            needs_me_reason,
        }
    }
}

/// The review bot, by login. It is a plain User account, so this is how it
/// is told apart from people.
fn is_review_bot(login: &str) -> bool {
    login.eq_ignore_ascii_case(BOT_LOGIN)
}

/// Human reviews as given, then pending requests. Someone with both a past
/// review and a pending request appears once, as requested (the author has
/// asked them to look again). The review bot and GitHub App accounts
/// (`is_bot`, e.g. "coderabbitai") are left out.
fn humans(pr: &GhPullRequest) -> Vec<HumanReview> {
    let requested = |login: &str| {
        pr.review_requests
            .iter()
            .any(|r| r.login.eq_ignore_ascii_case(login))
    };
    let mut out: Vec<HumanReview> = Vec::new();
    let seen =
        |out: &[HumanReview], login: &str| out.iter().any(|h| h.login.eq_ignore_ascii_case(login));

    for review in &pr.reviews {
        if review.is_bot
            || is_review_bot(&review.login)
            || requested(&review.login)
            || seen(&out, &review.login)
        {
            continue;
        }
        out.push(HumanReview {
            login: review.login.clone(),
            avatar_url: review.avatar_url.clone(),
            state: match review.state {
                GhReviewState::Approved => HumanReviewState::Approved,
                GhReviewState::ChangesRequested => HumanReviewState::ChangesRequested,
                GhReviewState::Commented => HumanReviewState::Commented,
                GhReviewState::Dismissed => HumanReviewState::Dismissed,
            },
        });
    }
    for request in &pr.review_requests {
        if request.is_bot || is_review_bot(&request.login) || seen(&out, &request.login) {
            continue;
        }
        let avatar_url = request.avatar_url.clone().or_else(|| {
            pr.reviews
                .iter()
                .find(|r| r.login.eq_ignore_ascii_case(&request.login))
                .and_then(|r| r.avatar_url.clone())
        });
        out.push(HumanReview {
            login: request.login.clone(),
            avatar_url,
            state: HumanReviewState::Requested,
        });
    }
    out
}

/// What merge has already worked out about a PR, for `needs_me`.
struct Judged<'a> {
    is_mine: bool,
    bot: &'a BotReview,
    humans: &'a [HumanReview],
    next_step: Option<&'a NextStep>,
}

/// Why the viewer has to act next on this PR, if they do. First match wins.
///
/// The viewer's own non-draft PRs: the bot requested changes, a person
/// requested changes, CI is failing, or the hygiene gate waits on their
/// self-review. Other people's PRs: the viewer's review was requested, or
/// the gate says it is ready for human review and names the viewer.
fn needs_me(pr: &GhPullRequest, judged: &Judged<'_>, viewer: Option<&str>) -> Option<String> {
    let Judged {
        is_mine,
        bot,
        humans,
        next_step,
    } = *judged;
    let stage = next_step.map(|s| s.stage.as_str());
    if is_mine {
        if pr.is_draft {
            return None;
        }
        if matches!(bot, BotReview::ChangesRequested { .. }) {
            return Some("Changes requested".to_string());
        }
        if let Some(h) = humans
            .iter()
            .find(|h| h.state == HumanReviewState::ChangesRequested)
        {
            return Some(format!("Changes requested by {}", h.login));
        }
        if pr.ci == CiState::Failing {
            return Some("CI failing".to_string());
        }
        if stage == Some("waiting-self-review") {
            return Some("Self-review needed".to_string());
        }
        return None;
    }
    let viewer = viewer?;
    if pr
        .review_requests
        .iter()
        .any(|r| r.login.eq_ignore_ascii_case(viewer))
    {
        return Some("Review requested".to_string());
    }
    let named = next_step
        .filter(|s| s.stage == "ready-for-human")
        .and_then(|s| s.detail.as_deref())
        .is_some_and(|detail| next_step::names_login(detail, viewer));
    named.then(|| "Your review is wanted".to_string())
}

fn feed_health(
    now: DateTime<Utc>,
    feed: Option<&FeedSnapshot>,
    status: SourceHealth,
) -> FeedHealth {
    let SourceHealth {
        mut ok,
        last_success_at,
        mut error,
    } = status;
    let Some(feed) = feed else {
        return FeedHealth {
            ok,
            last_success_at,
            error,
            ..FeedHealth::default()
        };
    };

    if let Some(as_of) = feed.data_as_of {
        let age = now.signed_duration_since(as_of);
        if age > TimeDelta::minutes(FEED_STALE_AFTER_MINUTES) {
            ok = false;
            // A failed fetch already says why; otherwise say what we see.
            if error.is_none() {
                error = Some(format!(
                    "The review feed has not updated for {} minutes.",
                    age.num_minutes()
                ));
            }
        }
    }

    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    FeedHealth {
        ok,
        last_success_at,
        error,
        data_as_of: feed.data_as_of,
        running: feed
            .heads_running
            .unwrap_or_else(|| count(feed.active.len())),
        queued: feed
            .heads_queued
            .unwrap_or_else(|| count(feed.queued.len())),
        median_review_minutes: feed.median_run_minutes,
        degraded: feed.degraded,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::feed::{FeedActive, FeedQueued, parse_feed};
    use crate::github::types::{
        GhBotCheck, GhCheckStatus, GhFinding, GhFindingReviewer, GhFindingStatus, GhHygiene,
        GhLastRounds, GhMergeFacts, GhMergeable, GhReview, GhReviewer, GhSeverity,
    };
    use crate::model::{
        NextActor, PrTrait, ReviewDecision, SelfReviewState, T3ThreadState, TraitKind, TraitLink,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    pub(crate) fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// A plain open PR by "someone" with passing CI and no reviews.
    pub(crate) fn gh_pr(repo: &str, number: u64) -> GhPullRequest {
        GhPullRequest {
            repo: repo.into(),
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/{repo}/pull/{number}"),
            author: "someone".into(),
            author_avatar_url: None,
            assignees: Vec::new(),
            milestone: None,
            is_draft: false,
            additions: 10,
            deletions: 2,
            created_at: at("2026-11-20T10:00:00Z"),
            updated_at: at("2026-11-21T10:00:00Z"),
            head_sha: "head".into(),
            head_branch: None,
            head_from_fork: false,
            review_decision: None,
            reviews: vec![],
            review_requests: vec![],
            ci: CiState::Passing,
            bot_check: None,
            hygiene: None,
            findings: Vec::new(),
            last_rounds: Default::default(),
            traits: Vec::new(),
            rounds: None,
            merge: Default::default(),
        }
    }

    pub(crate) fn check(
        status: GhCheckStatus,
        conclusion: Option<&str>,
        title: Option<&str>,
    ) -> GhBotCheck {
        GhBotCheck {
            status,
            conclusion: conclusion.map(Into::into),
            title: title.map(Into::into),
            started_at: None,
            completed_at: None,
        }
    }

    fn review(login: &str, state: GhReviewState) -> GhReview {
        GhReview {
            login: login.into(),
            avatar_url: Some(format!("https://avatars.test/{login}")),
            state,
            submitted_at: None,
            is_bot: false,
        }
    }

    fn reviewer(login: &str) -> GhReviewer {
        GhReviewer {
            login: login.into(),
            avatar_url: None,
            is_bot: false,
        }
    }

    fn app(mut r: GhReview) -> GhReview {
        r.is_bot = true;
        r
    }

    fn gate(description: &str) -> Option<GhHygiene> {
        Some(GhHygiene {
            state: "PENDING".into(),
            description: Some(description.into()),
        })
    }

    const NOW: &str = "2026-11-21T20:05:00Z";

    fn viewer() -> Viewer {
        Viewer {
            login: "Me".into(),
            avatar_url: None,
        }
    }

    fn project(repo: &str, prs: Vec<GhPullRequest>) -> ProjectInput {
        ProjectInput {
            repo: repo.into(),
            project: Some(GhProject {
                repo: repo.into(),
                open_count: 42,
                prs,
            }),
            error: None,
        }
    }

    fn healthy() -> SourceHealth {
        SourceHealth {
            ok: true,
            last_success_at: Some(at(NOW)),
            error: None,
        }
    }

    fn merge(
        config: &Config,
        projects: Vec<ProjectInput>,
        feed: Option<&FeedSnapshot>,
    ) -> Snapshot {
        build_snapshot(MergeInput {
            now: at(NOW),
            viewer: Some(viewer()),
            config,
            projects,
            github: healthy(),
            feed,
            feed_status: healthy(),
            t3: None,
            self_reviews: &[],
        })
    }

    /// Merges one PR with the default config and no feed.
    fn one(pr: GhPullRequest) -> PullRequest {
        let config = Config::default();
        let snap = merge(&config, vec![project("o/r", vec![pr])], None);
        snap.projects
            .into_iter()
            .next()
            .and_then(|p| p.prs.into_iter().next())
            .expect("one PR")
    }

    fn mine() -> GhPullRequest {
        let mut pr = gh_pr("o/r", 1);
        pr.author = "me".into();
        pr
    }

    #[test]
    fn ownership_and_following_ignore_case() {
        let config = Config {
            projects: vec!["o/r".into()],
            followed: vec!["O/R#2".into()],
            contributors: vec![],
        };
        let snap = merge(
            &config,
            vec![project(
                "o/r",
                vec![mine(), gh_pr("o/r", 2), gh_pr("o/r", 3)],
            )],
            None,
        );
        let flags: Vec<_> = snap.projects[0]
            .prs
            .iter()
            .map(|p| (p.id.as_str(), p.is_mine, p.is_followed))
            .collect();
        assert_eq!(
            flags,
            vec![
                ("o/r#1", true, false),
                ("o/r#2", false, true),
                ("o/r#3", false, false)
            ]
        );
    }

    /// A tracked contributor's PR is judged like any PR by someone else:
    /// tracking says nothing about whose it is or whether it is followed.
    #[test]
    fn a_tracked_contributors_pr_is_neither_mine_nor_followed() {
        let config = Config {
            projects: vec!["o/r".into()],
            followed: vec!["o/r#3".into()],
            contributors: vec!["Bob".into()],
        };
        let by_bob = |number: u64| {
            let mut pr = gh_pr("o/r", number);
            pr.author = "bob".into();
            // Their PR is in good shape for the viewer to merge.
            pr.merge = GhMergeFacts {
                node_id: format!("PR_{number}"),
                base_branch: "main".into(),
                review_threads: 1,
                unresolved_threads: 0,
                mergeable: GhMergeable::Mergeable,
                merge_state: "CLEAN".into(),
                squash_allowed: true,
                viewer_can_merge: true,
                merge_queue: false,
            };
            pr
        };
        // #1: plain. #2: asks for the viewer's review, with the gate waiting
        // on its author's self-review and CI failing. #3: also followed.
        let mut asks = by_bob(2);
        asks.review_requests = vec![reviewer("me")];
        asks.ci = CiState::Failing;
        asks.hygiene = gate("waiting-self-review: post /self-reviewed <sha>");
        let snap = merge(
            &config,
            vec![project("o/r", vec![by_bob(1), asks, by_bob(3)])],
            None,
        );
        let prs = &snap.projects[0].prs;
        let flags: Vec<_> = prs
            .iter()
            .map(|p| {
                (
                    p.number,
                    p.is_mine,
                    p.is_followed,
                    p.needs_me_reason.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            flags,
            vec![
                (1, false, false, None),
                (2, false, false, Some("Review requested")),
                (3, false, true, None),
            ]
        );
        for pr in prs {
            assert_eq!(
                pr.self_review.state,
                SelfReviewState::Unavailable,
                "#{}",
                pr.number
            );
            assert!(pr.merge.ready, "#{}: {:?}", pr.number, pr.merge.reason);
        }
        // Assigned to the viewer, they hold it: the self-review is theirs.
        let mut assigned = by_bob(4);
        assigned.assignees = vec!["Me".into()];
        assigned.hygiene = gate("waiting-self-review: post /self-reviewed <sha>");
        let got = one(assigned);
        assert!(!got.is_mine);
        assert_eq!(got.self_review.state, SelfReviewState::Wanted);
    }

    #[test]
    fn without_a_viewer_nothing_is_mine() {
        let config = Config::default();
        let snap = build_snapshot(MergeInput {
            now: at(NOW),
            viewer: None,
            config: &config,
            projects: vec![project("o/r", vec![mine()])],
            github: SourceHealth::default(),
            feed: None,
            feed_status: SourceHealth::default(),
            t3: None,
            self_reviews: &[],
        });
        assert!(!snap.projects[0].prs[0].is_mine);
        assert_eq!(snap.viewer, None);
    }

    #[test]
    fn humans_skip_the_bot_and_dedupe_requests() {
        let mut pr = gh_pr("o/r", 1);
        pr.reviews = vec![
            review("alice", GhReviewState::Approved),
            review("ThePastaClaw", GhReviewState::ChangesRequested),
            review("bob", GhReviewState::ChangesRequested),
            review("carol", GhReviewState::Dismissed),
        ];
        pr.review_requests = vec![reviewer("dave"), reviewer("Bob"), reviewer("thepastaclaw")];
        let got: Vec<_> = humans(&pr)
            .into_iter()
            .map(|h| (h.login, h.avatar_url, h.state))
            .collect();
        let avatar = |l: &str| Some(format!("https://avatars.test/{l}"));
        assert_eq!(
            got,
            vec![
                ("alice".into(), avatar("alice"), HumanReviewState::Approved),
                ("carol".into(), avatar("carol"), HumanReviewState::Dismissed),
                ("dave".into(), None, HumanReviewState::Requested),
                ("Bob".into(), avatar("bob"), HumanReviewState::Requested),
            ]
        );
    }

    #[test]
    fn humans_skip_github_app_accounts() {
        let mut pr = mine();
        pr.reviews = vec![
            app(review("coderabbitai", GhReviewState::ChangesRequested)),
            review("alice", GhReviewState::Commented),
        ];
        pr.review_requests = vec![
            GhReviewer {
                is_bot: true,
                ..reviewer("copilot-pull-request-reviewer")
            },
            reviewer("bob"),
        ];
        let got = one(pr);
        let logins: Vec<_> = got.humans.iter().map(|h| h.login.as_str()).collect();
        assert_eq!(logins, vec!["alice", "bob"]);
        // An app's "changes requested" is not a person's.
        assert!(!got.needs_me);
    }

    #[test]
    fn next_step_comes_from_the_gate() {
        let mut pr = mine();
        pr.hygiene = gate("waiting-bots: Waiting for bot review - thepastaclaw not yet.");
        let got = one(pr).next_step.expect("a next step");
        assert_eq!(got.stage, "waiting-bots");
        assert_eq!(got.label, "Waiting for bots");
        assert_eq!(got.actor, NextActor::Bots);

        // "Your move" on mine, "Author's move" on others'.
        let mut pr = mine();
        pr.hygiene = gate("waiting-self-review: Bots are done.");
        assert_eq!(one(pr).next_step.unwrap().label, "Your move");
        let mut pr = gh_pr("o/r", 2);
        pr.hygiene = gate("waiting-self-review: Bots are done.");
        assert_eq!(one(pr).next_step.unwrap().label, "Author's move");

        assert_eq!(one(mine()).next_step, None);
    }

    #[test]
    fn needs_me_self_review_on_my_pr() {
        let mut pr = mine();
        pr.hygiene = gate("waiting-self-review: Bots are done - your move: post /self-reviewed.");
        let got = one(pr);
        assert!(got.needs_me);
        assert_eq!(got.needs_me_reason.as_deref(), Some("Self-review needed"));
    }

    #[test]
    fn needs_me_ci_failing_beats_self_review() {
        let mut pr = mine();
        pr.hygiene = gate("waiting-self-review: Bots are done.");
        pr.ci = CiState::Failing;
        assert_eq!(one(pr).needs_me_reason.as_deref(), Some("CI failing"));
    }

    #[test]
    fn self_review_does_not_apply_to_drafts_or_others() {
        let mut pr = mine();
        pr.is_draft = true;
        pr.hygiene = gate("waiting-self-review: Bots are done.");
        assert!(!one(pr).needs_me);

        let mut pr = gh_pr("o/r", 2);
        pr.hygiene = gate("waiting-self-review: Bots are done.");
        assert!(!one(pr).needs_me);
    }

    #[test]
    fn other_stages_on_my_pr_do_not_need_me() {
        for d in [
            "waiting-bots: x",
            "ready-for-human: Ready for review - dpp: me",
            "waiting-build: x",
            "ready-to-merge",
        ] {
            let mut pr = mine();
            pr.hygiene = gate(d);
            assert!(!one(pr).needs_me, "{d}");
        }
    }

    #[test]
    fn needs_me_when_the_gate_names_me_as_reviewer() {
        let mut pr = gh_pr("o/r", 2);
        pr.hygiene = gate("ready-for-human: Ready for review - dpp: Alice or ME | sdk: bob");
        let got = one(pr);
        assert!(got.needs_me);
        assert_eq!(
            got.needs_me_reason.as_deref(),
            Some("Your review is wanted")
        );
    }

    #[test]
    fn a_request_beats_being_named_by_the_gate() {
        let mut pr = gh_pr("o/r", 2);
        pr.review_requests = vec![reviewer("me")];
        pr.hygiene = gate("ready-for-human: Ready for review - dpp: me");
        assert_eq!(one(pr).needs_me_reason.as_deref(), Some("Review requested"));
    }

    #[test]
    fn being_named_needs_a_whole_word_and_the_right_stage() {
        // The viewer "Me" is a prefix of "meadow" and part of "some-me".
        let mut pr = gh_pr("o/r", 2);
        pr.hygiene = gate("ready-for-human: Ready for review - dpp: meadow or some-me");
        assert!(!one(pr).needs_me);
        // Named, but not in the ready-for-human stage.
        let mut pr = gh_pr("o/r", 2);
        pr.hygiene = gate("waiting-bots: me");
        assert!(!one(pr).needs_me);
        // No detail at all.
        let mut pr = gh_pr("o/r", 2);
        pr.hygiene = gate("ready-for-human");
        assert!(!one(pr).needs_me);
    }

    #[test]
    fn needs_me_bot_changes_requested_first() {
        let mut pr = mine();
        pr.bot_check = Some(check(GhCheckStatus::Completed, Some("FAILURE"), None));
        pr.reviews = vec![review("bob", GhReviewState::ChangesRequested)];
        pr.ci = CiState::Failing;
        let got = one(pr);
        assert!(got.needs_me);
        assert_eq!(got.needs_me_reason.as_deref(), Some("Changes requested"));
    }

    #[test]
    fn needs_me_human_changes_requested() {
        let mut pr = mine();
        pr.reviews = vec![
            review("alice", GhReviewState::Approved),
            review("bob", GhReviewState::ChangesRequested),
        ];
        pr.ci = CiState::Failing;
        assert_eq!(
            one(pr).needs_me_reason.as_deref(),
            Some("Changes requested by bob")
        );
    }

    #[test]
    fn needs_me_ignores_changes_requested_once_re_requested() {
        let mut pr = mine();
        pr.reviews = vec![review("bob", GhReviewState::ChangesRequested)];
        pr.review_requests = vec![reviewer("bob")];
        let got = one(pr);
        assert!(!got.needs_me);
        assert_eq!(got.needs_me_reason, None);
    }

    #[test]
    fn needs_me_ci_failing() {
        let mut pr = mine();
        pr.ci = CiState::Failing;
        assert_eq!(one(pr).needs_me_reason.as_deref(), Some("CI failing"));
    }

    #[test]
    fn needs_me_review_requested_on_others_prs() {
        let mut pr = gh_pr("o/r", 1);
        pr.review_requests = vec![reviewer("ME")];
        pr.ci = CiState::Failing;
        let got = one(pr);
        assert!(got.needs_me);
        assert_eq!(got.needs_me_reason.as_deref(), Some("Review requested"));
    }

    #[test]
    fn others_prs_do_not_need_me_for_their_failures() {
        let mut pr = gh_pr("o/r", 1);
        pr.ci = CiState::Failing;
        pr.bot_check = Some(check(GhCheckStatus::Completed, Some("FAILURE"), None));
        assert!(!one(pr).needs_me);
    }

    #[test]
    fn my_drafts_never_need_me() {
        let mut pr = mine();
        pr.is_draft = true;
        pr.ci = CiState::Failing;
        pr.bot_check = Some(check(GhCheckStatus::Completed, Some("FAILURE"), None));
        pr.review_requests = vec![reviewer("me")];
        let got = one(pr);
        assert!(!got.needs_me);
        assert_eq!(got.needs_me_reason, None);
    }

    #[test]
    fn my_healthy_pr_does_not_need_me() {
        assert!(!one(mine()).needs_me);
    }

    #[test]
    fn projects_keep_order_errors_and_counts() {
        let config = Config::default();
        let snap = merge(
            &config,
            vec![
                project("a/one", vec![gh_pr("a/one", 1)]),
                ProjectInput {
                    repo: "b/two".into(),
                    project: None,
                    error: Some("not found: b/two".into()),
                },
                ProjectInput {
                    error: Some("network error: timeout".into()),
                    ..project("c/three", vec![gh_pr("c/three", 5)])
                },
            ],
            None,
        );
        let got: Vec<_> = snap
            .projects
            .iter()
            .map(|p| {
                (
                    p.repo.as_str(),
                    p.open_count,
                    p.prs.len(),
                    p.error.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("a/one", 42, 1, None),
                ("b/two", 0, 0, Some("not found: b/two")),
                ("c/three", 42, 1, Some("network error: timeout")),
            ]
        );
        assert_eq!(snap.generated_at, at(NOW));
        assert_eq!(snap.github, healthy());
    }

    #[test]
    fn rounds_are_the_bots_review_posts_read_from_github() {
        // Counted from GitHub for every PR, read or not; the feed has no say.
        let mut counted = gh_pr("o/r", 1);
        counted.rounds = Some(7);
        let mut none_yet = gh_pr("o/r", 2);
        none_yet.rounds = Some(0);
        let unread = gh_pr("o/r", 3);
        let feed = FeedSnapshot {
            data_as_of: Some(at(NOW)),
            ..FeedSnapshot::default()
        };
        let snap = merge(
            &Config::default(),
            vec![project("o/r", vec![counted, none_yet, unread])],
            Some(&feed),
        );
        let rounds: Vec<_> = snap.projects[0].prs.iter().map(|p| p.rounds).collect();
        assert_eq!(rounds, vec![Some(7), Some(0), None]);
    }

    #[test]
    fn feed_health_numbers() {
        let feed = FeedSnapshot {
            data_as_of: Some(at("2026-11-21T20:00:00Z")),
            heads_running: Some(5),
            heads_queued: None,
            queued: vec![FeedQueued::default(), FeedQueued::default()],
            active: vec![FeedActive::default()],
            median_run_minutes: Some(23.1),
            degraded: true,
            ..FeedSnapshot::default()
        };
        assert_eq!(
            feed_health(at(NOW), Some(&feed), healthy()),
            FeedHealth {
                ok: true,
                last_success_at: Some(at(NOW)),
                error: None,
                data_as_of: Some(at("2026-11-21T20:00:00Z")),
                running: 5,
                queued: 2,
                median_review_minutes: Some(23.1),
                degraded: true,
            }
        );
    }

    #[test]
    fn a_feed_half_an_hour_old_is_not_used_for_live_state() {
        // The feed still lists the PR as running, but stopped updating 35
        // minutes ago; GitHub's check run is all we can trust.
        let mut pr = gh_pr("o/r", 1);
        pr.bot_check = Some(check(
            GhCheckStatus::InProgress,
            None,
            Some("Reviewing — Phase 1 review"),
        ));
        let feed = FeedSnapshot {
            data_as_of: Some(at("2026-11-21T19:30:00Z")),
            active: vec![crate::feed::FeedActive {
                repo: "o/r".into(),
                number: 1,
                phase: Some("phase2".into()),
                progress: Some(0.9),
                ..Default::default()
            }],
            ..FeedSnapshot::default()
        };
        let config = Config::default();
        let snap = merge(&config, vec![project("o/r", vec![pr])], Some(&feed));
        match &snap.projects[0].prs[0].bot {
            BotReview::Reviewing {
                phase, progress, ..
            } => {
                assert_eq!(phase, "unknown");
                assert_eq!(*progress, None);
            }
            other => panic!("expected reviewing from the check run, got {other:?}"),
        }
    }

    #[test]
    fn a_renamed_repo_keeps_the_name_the_user_tracks() {
        // The config says old/name; GitHub now answers new/name.
        let config = Config {
            projects: vec!["old/name".into()],
            followed: vec!["old/name#12".into()],
            contributors: vec![],
        };
        let feed = FeedSnapshot {
            data_as_of: Some(at(NOW)),
            ..FeedSnapshot::default()
        };
        let mut renamed = gh_pr("new/name", 12);
        renamed.rounds = Some(4);
        let snap = merge(
            &config,
            vec![project("old/name", vec![renamed])],
            Some(&feed),
        );
        let pr = &snap.projects[0].prs[0];
        assert_eq!(snap.projects[0].repo, "old/name");
        assert_eq!(pr.repo, "old/name");
        assert_eq!(pr.id, "old/name#12");
        assert!(pr.is_followed);
        assert_eq!(pr.rounds, Some(4));
    }

    #[test]
    fn a_stale_feed_is_unhealthy() {
        let feed = FeedSnapshot {
            data_as_of: Some(at("2026-11-21T19:51:00Z")),
            ..FeedSnapshot::default()
        };
        let health = feed_health(at(NOW), Some(&feed), healthy());
        assert!(!health.ok);
        assert_eq!(
            health.error.as_deref(),
            Some("The review feed has not updated for 14 minutes.")
        );
        assert_eq!(health.last_success_at, Some(at(NOW)));
    }

    #[test]
    fn ten_minutes_old_is_still_fresh() {
        let feed = FeedSnapshot {
            data_as_of: Some(at("2026-11-21T19:55:00Z")),
            ..FeedSnapshot::default()
        };
        assert!(feed_health(at(NOW), Some(&feed), healthy()).ok);
    }

    #[test]
    fn a_failed_fetch_keeps_its_own_error() {
        let feed = FeedSnapshot {
            data_as_of: Some(at("2026-11-21T19:00:00Z")),
            ..FeedSnapshot::default()
        };
        let failed = SourceHealth {
            ok: false,
            last_success_at: Some(at("2026-11-21T19:00:30Z")),
            error: Some("network error: timed out".into()),
        };
        let health = feed_health(at(NOW), Some(&feed), failed);
        assert!(!health.ok);
        assert_eq!(health.error.as_deref(), Some("network error: timed out"));
        assert_eq!(health.data_as_of, Some(at("2026-11-21T19:00:00Z")));
    }

    #[test]
    fn no_feed_has_no_numbers() {
        let failed = SourceHealth {
            ok: false,
            last_success_at: None,
            error: Some("network error: offline".into()),
        };
        assert_eq!(
            feed_health(at(NOW), None, failed),
            FeedHealth {
                ok: false,
                last_success_at: None,
                error: Some("network error: offline".into()),
                data_as_of: None,
                running: 0,
                queued: 0,
                median_review_minutes: None,
                degraded: false,
            }
        );
    }

    #[test]
    fn merges_the_real_feed_fixture() {
        let path = format!(
            "{}/tests/fixtures/feed/real_trimmed.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let feed = parse_feed(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut reviewing = gh_pr("dashpay/dash-platform-website", 310);
        reviewing.head_sha = feed.active[1].sha.clone().unwrap();
        let mut waiting = gh_pr("dashpay/dashwallet-ios", 1089);
        waiting.head_sha = feed.queued[0].sha.clone().unwrap();
        let config = Config::default();
        let snap = build_snapshot(MergeInput {
            now: at("2026-11-21T20:07:00Z"),
            viewer: Some(viewer()),
            config: &config,
            projects: vec![project("dashpay/x", vec![reviewing, waiting])],
            github: healthy(),
            feed: Some(&feed),
            feed_status: healthy(),
            t3: None,
            self_reviews: &[],
        });
        let prs = &snap.projects[0].prs;

        let BotReview::Reviewing {
            phase_label,
            finishes_at,
            steps,
            ..
        } = &prs[0].bot
        else {
            panic!("expected reviewing, got {:?}", prs[0].bot);
        };
        assert_eq!(phase_label, "Phase 2");
        assert!(finishes_at.is_some());
        let keys: Vec<_> = steps.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "triage", "phase1", "verify1", "gate", "phase2", "verify2", "publish"
            ]
        );
        assert_eq!(steps[4].detail.as_deref(), Some("0 of 2 lanes"));
        // The feed's `rounds` section is not read: rounds come from GitHub.
        assert_eq!(prs[0].rounds, None);

        assert_eq!(
            prs[1].bot,
            BotReview::Queued {
                position: Some(1),
                starts_at: Some(at("2026-11-21T20:33:56Z")),
                // 20:33:56 + 22 s slot wait + 1914 s duration.
                finishes_at: Some(at("2026-11-21T21:06:12Z")),
                reason: Some("debounce (more pushes may follow)".into()),
                priority: false,
            }
        );
        assert_eq!(prs[1].rounds, None);
        assert!(snap.feed.ok);
        assert_eq!(snap.feed.running, 5);
    }

    #[test]
    fn snapshot_serializes_as_camel_case_with_a_state_tag() {
        let mut pr = mine();
        pr.review_decision = Some(ReviewDecision::ReviewRequired);
        pr.review_requests = vec![reviewer("alice")];
        pr.bot_check = Some(check(GhCheckStatus::Queued, None, None));
        pr.hygiene = gate("ready-for-human: Ready for review - dpp: alice");
        let mut bot_review = review(BOT_LOGIN, GhReviewState::Commented);
        bot_review.submitted_at = Some(at("2026-11-21T19:30:00Z"));
        pr.reviews = vec![bot_review];
        pr.last_rounds = GhLastRounds {
            review_bot: Some(at("2026-11-21T18:00:00Z")),
            coderabbit: Some(at("2026-11-21T19:00:00Z")),
            review_bot_commit: Some("older".into()),
            coderabbit_commit: Some("head".into()),
        };
        pr.findings = vec![
            GhFinding {
                reviewer: GhFindingReviewer::ReviewBot,
                severity: GhSeverity::Blocking,
                status: GhFindingStatus::Resolved,
                thread_resolved: true,
            },
            GhFinding {
                reviewer: GhFindingReviewer::CodeRabbit,
                severity: GhSeverity::Blocking,
                status: GhFindingStatus::Stands,
                thread_resolved: false,
            },
            GhFinding {
                reviewer: GhFindingReviewer::ReviewBot,
                severity: GhSeverity::Suggestion,
                status: GhFindingStatus::Disputed,
                thread_resolved: true,
            },
            // Resolved by hand after the bot's round on an older commit.
            GhFinding {
                reviewer: GhFindingReviewer::ReviewBot,
                severity: GhSeverity::Blocking,
                status: GhFindingStatus::Stands,
                thread_resolved: true,
            },
        ];
        pr.traits = vec![PrTrait {
            kind: TraitKind::ContractChange,
            summary: "dpns-contract v3: changed".into(),
            links: vec![TraitLink {
                label: "dpns-contract v3 (compare changes)".into(),
                url: "https://dashpay.github.io/contract-visualizer/?base=a&head=b".into(),
            }],
        }];
        pr.merge = GhMergeFacts {
            node_id: "PR_kw1".into(),
            base_branch: "main".into(),
            review_threads: 5,
            unresolved_threads: 2,
            mergeable: GhMergeable::Mergeable,
            merge_state: "BLOCKED".into(),
            squash_allowed: true,
            viewer_can_merge: true,
            merge_queue: false,
        };
        let links = vec![T3Link {
            repo: "O/R".into(),
            number: 1,
            thread_id: "thread-1".into(),
            title: "Fix the flaky test".into(),
            state: T3ThreadState::Waiting,
            watching: true,
            last_active_at: Some(at("2026-11-21T20:00:00Z")),
        }];
        let config = Config::default();
        let snap = build_snapshot(MergeInput {
            now: at(NOW),
            viewer: Some(viewer()),
            config: &config,
            projects: vec![project("o/r", vec![pr])],
            github: healthy(),
            feed: None,
            feed_status: SourceHealth::default(),
            t3: Some(&links),
            self_reviews: &[],
        });
        let pr_json = json!({
        "id": "o/r#1",
        "repo": "o/r",
        "number": 1,
        "title": "PR 1",
        "url": "https://github.com/o/r/pull/1",
        "author": "me",
        "authorAvatarUrl": null,
        "milestone": null,
        "isDraft": false,
        "isMine": true,
        "isFollowed": false,
        "additions": 10,
        "deletions": 2,
        "createdAt": "2026-11-20T10:00:00Z",
        "updatedAt": "2026-11-21T10:00:00Z",
        "headSha": "head",
        "headBranch": null,
        "ci": "passing",
        "reviewDecision": "reviewRequired",
        "humans": [{"login": "alice", "avatarUrl": null, "state": "requested"}],
        "bot": {
            "state": "queued",
            "position": null,
            "startsAt": null,
            "finishesAt": null,
            "reason": null,
            "priority": false
        },
        "rounds": null,
        "nextStep": {
            "stage": "ready-for-human",
            "label": "Ready for human review",
            "detail": "Ready for review - dpp: alice",
            "actor": "reviewers"
        },
        "lastReviewAt": "2026-11-21T19:00:00Z",
        "findingCounts": {
            "blocking": {"total": 3, "agreed": 1, "notAgreed": 0, "awaiting": 1, "coderabbit": 1},
            "suggestion": {"total": 1, "agreed": 0, "notAgreed": 1, "awaiting": 0, "coderabbit": 0},
            "nitpick": {"total": 0, "agreed": 0, "notAgreed": 0, "awaiting": 0, "coderabbit": 0}
        },
        "traits": [{
            "kind": "contractChange",
            "summary": "dpns-contract v3: changed",
            "links": [{
                "label": "dpns-contract v3 (compare changes)",
                "url": "https://dashpay.github.io/contract-visualizer/?base=a&head=b"
            }]
        }],
        "baseBranch": "main",
        "stackedOn": null,
        "unresolvedThreads": 2,
        "merge": {"ready": false, "reason": "2 unresolved comments"},
        "hasConflicts": false,
        "selfReview": {"state": "unavailable", "postedAt": null},
        "t3Threads": [{
            "threadId": "thread-1",
            "title": "Fix the flaky test",
            "state": "waiting",
            "lastActiveAt": "2026-11-21T20:00:00Z",
            "watching": true
        }],
        "needsMe": false,
        "needsMeReason": null
        });
        let expected = json!({
            "generatedAt": "2026-11-21T20:05:00Z",
            "signInRequired": false,
            "t3Available": true,
            "viewer": {"login": "Me", "avatarUrl": null},
            "projects": [{
                "repo": "o/r",
                "openCount": 42,
                "error": null,
                "prs": [pr_json]
            }],
            "github": {"ok": true, "lastSuccessAt": "2026-11-21T20:05:00Z", "error": null},
            "feed": {
                "ok": false,
                "lastSuccessAt": null,
                "error": null,
                "dataAsOf": null,
                "running": 0,
                "queued": 0,
                "medianReviewMinutes": null,
                "degraded": false
            }
        });
        assert_eq!(serde_json::to_value(&snap).unwrap(), expected);
    }

    #[test]
    fn stacked_on_links_prs_of_one_project_whatever_view_they_are_in() {
        let branch = |number: u64, head: &str, base: &str, author: &str| {
            let mut pr = gh_pr("o/r", number);
            pr.author = author.into();
            pr.head_branch = Some(head.into());
            pr.merge.base_branch = base.into();
            pr
        };
        // Mine at the bottom, a followed PR on it, a contributor's on that.
        let config = Config {
            followed: vec!["o/r#2".into()],
            ..Config::default()
        };
        let mut other = gh_pr("o/s", 4);
        other.head_branch = Some("feat/c".into());
        other.merge.base_branch = "feat/a".into();
        let snap = merge(
            &config,
            vec![
                project(
                    "o/r",
                    vec![
                        branch(3, "feat/c", "feat/b", "alice"),
                        branch(1, "feat/a", "main", "me"),
                        branch(2, "feat/b", "feat/a", "bob"),
                    ],
                ),
                // Same branch names in another project link to nothing there.
                project("o/s", vec![other]),
            ],
            None,
        );
        let got: Vec<(String, Option<u64>)> = snap
            .projects
            .iter()
            .flat_map(|p| &p.prs)
            .map(|pr| (pr.id.clone(), pr.stacked_on))
            .collect();
        assert_eq!(
            got,
            vec![
                ("o/r#3".to_owned(), Some(2)),
                ("o/r#1".to_owned(), None),
                ("o/r#2".to_owned(), Some(1)),
                ("o/s#4".to_owned(), None),
            ]
        );
    }

    #[test]
    fn merge_readiness_and_t3_threads_reach_each_pr() {
        let mut ready = gh_pr("o/r", 1);
        ready.merge = GhMergeFacts {
            node_id: "PR_1".into(),
            base_branch: "main".into(),
            mergeable: GhMergeable::Mergeable,
            merge_state: "CLEAN".into(),
            squash_allowed: true,
            viewer_can_merge: true,
            ..GhMergeFacts::default()
        };
        let mut draft = ready.clone();
        draft.number = 2;
        draft.is_draft = true;
        // Conflicts too: the merge reason names the draft first, the flag still says it.
        draft.merge.mergeable = GhMergeable::Conflicting;
        let links = vec![
            T3Link {
                repo: "o/r".into(),
                number: 2,
                thread_id: "t".into(),
                title: "Draft work".into(),
                state: T3ThreadState::Idle,
                watching: false,
                last_active_at: None,
            },
            T3Link {
                repo: "o/other".into(),
                number: 1,
                thread_id: "elsewhere".into(),
                title: "Wrong repo".into(),
                state: T3ThreadState::Working,
                watching: false,
                last_active_at: None,
            },
        ];
        let config = Config::default();
        let build = |t3: Option<&[T3Link]>| {
            build_snapshot(MergeInput {
                now: at(NOW),
                viewer: Some(viewer()),
                config: &config,
                projects: vec![project("o/r", vec![ready.clone(), draft.clone()])],
                github: healthy(),
                feed: None,
                feed_status: healthy(),
                t3,
                self_reviews: &[],
            })
        };
        let snap = build(Some(&links));
        assert!(snap.t3_available);
        let prs = &snap.projects[0].prs;
        assert!(prs[0].merge.ready);
        assert!(!prs[0].has_conflicts);
        assert_eq!(prs[0].base_branch, "main");
        assert!(prs[1].has_conflicts);
        assert!(prs[0].t3_threads.is_empty());
        assert_eq!(
            prs[1].merge.reason.as_deref(),
            Some("Draft pull requests cannot be merged.")
        );
        assert_eq!(prs[1].t3_threads.len(), 1);
        assert_eq!(prs[1].t3_threads[0].thread_id, "t");

        let snap = build(None);
        assert!(!snap.t3_available);
        assert!(snap.projects[0].prs.iter().all(|p| p.t3_threads.is_empty()));
    }

    #[test]
    fn reviewing_serializes_its_fields_in_camel_case() {
        let bot = BotReview::Reviewing {
            phase: "phase1".into(),
            phase_label: "Phase 1".into(),
            progress: Some(0.5),
            finishes_at: Some(at(NOW)),
            started_at: None,
            overdue: false,
            tier: None,
            priority: true,
            waited_seconds: Some(3),
            steps: vec![],
            findings: None,
        };
        assert_eq!(
            serde_json::to_value(&bot).unwrap(),
            json!({
                "state": "reviewing",
                "phase": "phase1",
                "phaseLabel": "Phase 1",
                "progress": 0.5,
                "finishesAt": "2026-11-21T20:05:00Z",
                "startedAt": null,
                "overdue": false,
                "tier": null,
                "priority": true,
                "waitedSeconds": 3,
                "steps": [],
                "findings": null
            })
        );
    }

    #[test]
    fn the_snapshot_carries_each_prs_self_review() {
        use crate::model::{SelfReview, SelfReviewState};
        let asks = GhHygiene {
            state: "PENDING".into(),
            description: Some(
                "waiting-self-review: Bots are done - your move: post /self-reviewed.".into(),
            ),
        };
        let mut posted_on = mine();
        posted_on.hygiene = Some(asks.clone());
        let mut assigned = gh_pr("o/r", 2);
        assigned.assignees = vec!["me".into()];
        assigned.hygiene = Some(GhHygiene {
            state: "PENDING".into(),
            description: Some("waiting-bots: Waiting for bot review.".into()),
        });
        let mut theirs = gh_pr("o/r", 3);
        theirs.hygiene = Some(asks.clone());
        let mut asked = mine();
        asked.number = 4;
        asked.hygiene = Some(asks);
        let config = Config::default();
        let ours = [PostedSelfReview {
            repo: "o/r".into(),
            number: 1,
            head_sha: "head".into(),
            at: at(NOW),
        }];
        let snap = build_snapshot(MergeInput {
            now: at(NOW),
            viewer: Some(viewer()),
            config: &config,
            projects: vec![project("o/r", vec![posted_on, assigned, theirs, asked])],
            github: healthy(),
            feed: None,
            feed_status: healthy(),
            t3: None,
            self_reviews: &ours,
        });
        let got: Vec<_> = snap.projects[0]
            .prs
            .iter()
            .map(|p| (p.number, p.self_review.clone()))
            .collect();
        let plain = |state| SelfReview {
            state,
            posted_at: None,
        };
        assert_eq!(
            got,
            vec![
                (
                    1,
                    SelfReview {
                        state: SelfReviewState::Posted,
                        posted_at: Some(at(NOW)),
                    }
                ),
                (2, plain(SelfReviewState::Possible)),
                (3, plain(SelfReviewState::Unavailable)),
                (4, plain(SelfReviewState::Wanted)),
            ]
        );
    }
}
