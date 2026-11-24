//! Which PR each PR is stacked on: the PR whose head branch is its base
//! branch, so that it merges into that PR's branch rather than into a
//! long-lived one. Pure; decided per project over the PRs of that project in
//! the snapshot (the viewer's, the followed ones and the tracked
//! contributors' alike).
//!
//! A child points at a parent when:
//! - both are in the same repo and are different PRs;
//! - the parent's head branch is exactly the child's base branch (branch
//!   names are case-sensitive; an empty or missing branch never matches);
//! - the parent's head is not from a fork (a fork's branch can never be a
//!   base in the project itself);
//! - exactly one such non-fork PR has that head branch (otherwise the link is
//!   ambiguous and dropped);
//! - the branch is the base of at most `MAX_STACK_CHILDREN` PRs (see there).
//!
//! Finally, links that form a cycle are dropped for every PR in the cycle.

use std::collections::{HashMap, HashSet};

use crate::github::types::GhPullRequest;

/// A branch that more PRs than this merge into is shared, not a stack: none
/// of those PRs is stacked on the PR whose head it is. Without this a
/// release PR such as `v5.1-dev → master` would swallow every PR that
/// targets `v5.1-dev`. A real stack rarely has more than a couple of PRs on
/// one parent, so three keeps those and drops the shared branches.
pub(super) const MAX_STACK_CHILDREN: usize = 3;

/// The number of the PR each of `prs` is stacked on, in the same order as
/// `prs`. The same PR listed twice counts once.
pub(super) fn stacked_on(prs: &[GhPullRequest]) -> Vec<Option<u64>> {
    // Repo names are not case-sensitive on GitHub; branch names are.
    let repos: Vec<String> = prs.iter().map(|p| p.repo.to_lowercase()).collect();

    // Non-fork PRs by head branch, and PRs by base branch, distinct numbers.
    let mut heads: HashMap<(&str, &str), HashSet<u64>> = HashMap::new();
    let mut bases: HashMap<(&str, &str), HashSet<u64>> = HashMap::new();
    for (pr, repo) in prs.iter().zip(&repos) {
        if let Some(head) = branch(pr.head_branch.as_deref())
            && !pr.head_from_fork
        {
            heads
                .entry((repo.as_str(), head))
                .or_default()
                .insert(pr.number);
        }
        if let Some(base) = branch(Some(&pr.merge.base_branch)) {
            bases
                .entry((repo.as_str(), base))
                .or_default()
                .insert(pr.number);
        }
    }

    let parent_of = |pr: &GhPullRequest, repo: &str| -> Option<u64> {
        let base = branch(Some(&pr.merge.base_branch))?;
        let key = (repo, base);
        if bases.get(&key).map_or(0, HashSet::len) > MAX_STACK_CHILDREN {
            return None;
        }
        let owners = heads.get(&key)?;
        if owners.len() != 1 {
            return None;
        }
        owners.iter().copied().find(|&n| n != pr.number)
    };

    let mut links: HashMap<(&str, u64), u64> = HashMap::new();
    for (pr, repo) in prs.iter().zip(&repos) {
        if let Some(parent) = parent_of(pr, repo) {
            links.insert((repo.as_str(), pr.number), parent);
        }
    }

    let in_cycle: HashSet<(&str, u64)> = links
        .keys()
        .copied()
        .filter(|&start| returns_to(&links, start))
        .collect();
    for key in &in_cycle {
        links.remove(key);
    }

    prs.iter()
        .zip(&repos)
        .map(|(pr, repo)| links.get(&(repo.as_str(), pr.number)).copied())
        .collect()
}

/// A branch name that can match: present and not blank.
fn branch(name: Option<&str>) -> Option<&str> {
    name.filter(|b| !b.trim().is_empty())
}

/// Following the links from `start` comes back to `start`. Every PR has at
/// most one parent, so a walk longer than the number of links is in a cycle
/// that `start` is not part of.
fn returns_to(links: &HashMap<(&str, u64), u64>, start: (&str, u64)) -> bool {
    let (repo, mut at) = start;
    for _ in 0..links.len() {
        match links.get(&(repo, at)) {
            Some(&parent) if parent == start.1 => return true,
            Some(&parent) => at = parent,
            None => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::tests::gh_pr;
    use pretty_assertions::assert_eq;

    const REPO: &str = "dashpay/platform";

    /// PR `number` from `head` into `base`, in `REPO`.
    fn pr(number: u64, head: &str, base: &str) -> GhPullRequest {
        let mut pr = gh_pr(REPO, number);
        pr.head_branch = Some(head.into()).filter(|h: &String| !h.is_empty());
        pr.merge.base_branch = base.into();
        pr
    }

    fn fork(mut pr: GhPullRequest) -> GhPullRequest {
        pr.head_from_fork = true;
        pr
    }

    #[test]
    fn a_chain_of_three_links_each_to_the_one_below() {
        let prs = [
            pr(
                6133,
                "feat/token-spend-ui",
                "feat/swift-example-shielded-token-spend",
            ),
            pr(6116, "feat/swift-example-shielded", "v3.1-dev"),
            pr(
                6132,
                "feat/swift-example-shielded-token-spend",
                "feat/swift-example-shielded",
            ),
        ];
        assert_eq!(stacked_on(&prs), vec![Some(6132), None, Some(6116)]);
    }

    #[test]
    fn a_parent_can_have_two_children() {
        let prs = [
            pr(5750, "feat/a", "v3.1-dev"),
            pr(5751, "feat/a-1", "feat/a"),
            pr(5753, "feat/a-2", "feat/a"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, Some(5750), Some(5750)]);
    }

    #[test]
    fn three_children_still_stack_but_four_make_a_shared_branch() {
        let mut prs = vec![
            pr(1, "v5.1-dev", "master"),
            pr(2, "fix/a", "v5.1-dev"),
            pr(3, "fix/b", "v5.1-dev"),
            pr(4, "fix/c", "v5.1-dev"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, Some(1), Some(1), Some(1)]);

        prs.push(pr(5, "fix/d", "v5.1-dev"));
        assert_eq!(stacked_on(&prs), vec![None; 5]);
    }

    #[test]
    fn a_pr_listed_twice_counts_once() {
        let prs = [
            pr(1, "feat/a", "main"),
            pr(1, "feat/a", "main"),
            pr(2, "feat/a-1", "feat/a"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, Some(1)]);
    }

    #[test]
    fn a_fork_head_is_never_a_parent_but_a_fork_pr_can_be_a_child() {
        let prs = [
            fork(pr(1, "feat/a", "main")),
            pr(2, "feat/a-1", "feat/a"),
            pr(3, "feat/b", "main"),
            fork(pr(4, "feat/b-1", "feat/b")),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, None, Some(3)]);
    }

    #[test]
    fn a_fork_with_the_same_branch_name_does_not_make_it_ambiguous() {
        let prs = [
            pr(1, "feat/a", "main"),
            fork(pr(2, "feat/a", "main")),
            pr(3, "feat/a-1", "feat/a"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, Some(1)]);
    }

    #[test]
    fn two_prs_with_the_same_head_branch_are_ambiguous() {
        let prs = [
            pr(1, "feat/a", "main"),
            pr(2, "feat/a", "v3.1-dev"),
            pr(3, "feat/a-1", "feat/a"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, None]);
    }

    #[test]
    fn a_two_pr_cycle_is_dropped_for_both_and_what_hangs_off_it_stays() {
        let prs = [
            pr(1, "feat/a", "feat/b"),
            pr(2, "feat/b", "feat/a"),
            pr(3, "feat/c", "feat/a"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, Some(1)]);
    }

    #[test]
    fn a_longer_cycle_is_dropped_for_every_pr_in_it() {
        let prs = [
            pr(1, "a", "c"),
            pr(2, "b", "a"),
            pr(3, "c", "b"),
            pr(4, "d", "c"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, None, Some(3)]);
    }

    #[test]
    fn a_pr_is_never_stacked_on_itself() {
        let prs = [pr(1, "feat/a", "feat/a")];
        assert_eq!(stacked_on(&prs), vec![None]);
    }

    #[test]
    fn empty_or_missing_branches_never_match() {
        let mut blank = pr(3, "feat/c", "  ");
        blank.head_branch = Some("  ".into());
        let prs = [
            pr(1, "", "main"),
            pr(2, "feat/b", ""),
            blank,
            pr(4, "feat/d", "  "),
        ];
        assert_eq!(stacked_on(&prs), vec![None; 4]);
    }

    #[test]
    fn branch_names_match_exactly() {
        let prs = [pr(1, "Feat/A", "main"), pr(2, "feat/a-1", "feat/a")];
        assert_eq!(stacked_on(&prs), vec![None, None]);
    }

    #[test]
    fn a_pr_in_another_repo_is_not_a_parent() {
        let mut other = pr(1, "feat/a", "main");
        other.repo = "dashpay/dips".into();
        let mut same_repo_other_case = pr(3, "feat/b", "main");
        same_repo_other_case.repo = "DashPay/Platform".into();
        let prs = [
            other,
            pr(2, "feat/a-1", "feat/a"),
            same_repo_other_case,
            pr(4, "feat/b-1", "feat/b"),
        ];
        assert_eq!(stacked_on(&prs), vec![None, None, None, Some(3)]);
    }
}
