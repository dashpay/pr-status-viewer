//! Which T3 Code threads belong to which PR.

use crate::model::{T3Thread, T3ThreadState};
use crate::t3::T3Link;

/// The T3 threads tied to PR `number`. A link matches when its repo equals,
/// ignoring case, either the PR's repo as GitHub names it (`gh_repo`) or the
/// project as the user tracks it (`tracked_repo`); the two differ after a
/// rename. One entry per thread: working ones first, then waiting, then
/// idle; within each the most recently active first, then by id so the
/// order is stable.
pub fn t3_threads(
    links: &[T3Link],
    tracked_repo: &str,
    gh_repo: &str,
    number: u64,
) -> Vec<T3Thread> {
    let mut out: Vec<T3Thread> = Vec::new();
    for link in links.iter().filter(|l| {
        l.number == number
            && (l.repo.eq_ignore_ascii_case(gh_repo) || l.repo.eq_ignore_ascii_case(tracked_repo))
    }) {
        match out.iter_mut().find(|t| t.thread_id == link.thread_id) {
            // The same thread listed twice: keep what is most alive and
            // most recent.
            Some(seen) => {
                if rank(link.state) < rank(seen.state) {
                    seen.state = link.state;
                }
                seen.watching |= link.watching;
                if link.last_active_at > seen.last_active_at {
                    seen.last_active_at = link.last_active_at;
                    seen.title.clone_from(&link.title);
                }
            }
            None => out.push(T3Thread {
                thread_id: link.thread_id.clone(),
                title: link.title.clone(),
                state: link.state,
                last_active_at: link.last_active_at,
                watching: link.watching,
            }),
        }
    }
    // `None` sorts below any time, so never-run threads come last.
    out.sort_by(|a, b| {
        rank(a.state)
            .cmp(&rank(b.state))
            .then_with(|| b.last_active_at.cmp(&a.last_active_at))
            .then_with(|| a.thread_id.cmp(&b.thread_id))
    });
    out
}

/// Most alive first.
fn rank(state: T3ThreadState) -> u8 {
    match state {
        T3ThreadState::Working => 0,
        T3ThreadState::Waiting => 1,
        T3ThreadState::Idle => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::tests::at;
    use pretty_assertions::assert_eq;

    fn link(repo: &str, number: u64, id: &str, state: T3ThreadState, at_: Option<&str>) -> T3Link {
        T3Link {
            repo: repo.into(),
            number,
            thread_id: id.into(),
            title: format!("thread {id}"),
            state,
            watching: false,
            last_active_at: at_.map(at),
        }
    }

    use T3ThreadState::{Idle, Waiting, Working};

    fn ids(threads: &[T3Thread]) -> Vec<&str> {
        threads.iter().map(|t| t.thread_id.as_str()).collect()
    }

    #[test]
    fn matches_number_and_either_repo_name_ignoring_case() {
        let links = vec![
            link("Acme/App", 7, "a", Idle, None),
            link("acme/old-name", 7, "b", Idle, None),
            link("acme/app", 8, "c", Idle, None),
            link("other/app", 7, "d", Idle, None),
        ];
        let got = t3_threads(&links, "acme/old-name", "ACME/app", 7);
        assert_eq!(ids(&got), vec!["a", "b"]);
        assert!(t3_threads(&links, "x/y", "x/y", 7).is_empty());
        assert!(t3_threads(&[], "acme/app", "acme/app", 7).is_empty());
    }

    #[test]
    fn working_then_waiting_then_idle_each_most_recent_first() {
        let links = vec![
            link("o/r", 1, "never", Idle, None),
            link("o/r", 1, "old", Idle, Some("2026-11-20T10:00:00Z")),
            link("o/r", 1, "new", Idle, Some("2026-11-22T10:00:00Z")),
            link("o/r", 1, "busy-old", Working, Some("2026-11-01T10:00:00Z")),
            link("o/r", 1, "busy-new", Working, Some("2026-11-22T11:00:00Z")),
            link("o/r", 1, "wait-old", Waiting, Some("2026-11-02T10:00:00Z")),
            link("o/r", 1, "wait-new", Waiting, Some("2026-11-23T10:00:00Z")),
        ];
        let got = t3_threads(&links, "o/r", "o/r", 1);
        assert_eq!(
            ids(&got),
            vec![
                "busy-new", "busy-old", "wait-new", "wait-old", "new", "old", "never"
            ]
        );
        assert_eq!(got[0].state, Working);
        assert_eq!(got[2].state, Waiting);
        assert_eq!(got[4].state, Idle);
        assert_eq!(got[4].title, "thread new");
        assert_eq!(got[4].last_active_at, Some(at("2026-11-22T10:00:00Z")));
    }

    #[test]
    fn watching_is_carried_through() {
        let mut watched = link("o/r", 1, "w", Waiting, None);
        watched.watching = true;
        let got = t3_threads(&[watched, link("o/r", 1, "u", Idle, None)], "o/r", "o/r", 1);
        assert_eq!(ids(&got), vec!["w", "u"]);
        assert!(got[0].watching);
        assert!(!got[1].watching);
    }

    #[test]
    fn one_entry_per_thread() {
        let mut later = link("O/R", 1, "t", Idle, Some("2026-11-22T10:00:00Z"));
        later.title = "renamed".into();
        later.watching = true;
        let links = vec![
            link("o/r", 1, "t", Waiting, Some("2026-11-21T10:00:00Z")),
            later,
            link("o/r", 1, "u", Idle, None),
            link("o/r", 1, "v", Waiting, None),
            link("o/r", 1, "v", Working, None),
        ];
        let got = t3_threads(&links, "o/r", "o/r", 1);
        assert_eq!(ids(&got), vec!["v", "t", "u"]);
        assert_eq!(got[0].state, Working);
        assert_eq!(got[1].state, Waiting);
        assert!(got[1].watching);
        assert_eq!(got[1].title, "renamed");
        assert_eq!(got[1].last_active_at, Some(at("2026-11-22T10:00:00Z")));
    }
}
