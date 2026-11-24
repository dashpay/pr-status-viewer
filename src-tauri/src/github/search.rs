//! The search strings that find the open PRs the viewer and the tracked
//! contributors wrote in one project. Pure.
//!
//! GitHub ORs repeated `author:` qualifiers, so one search covers the viewer
//! (`author:@me`) and several contributors at the same rate-limit cost as the
//! viewer alone. GitHub documents a 256-character limit on search queries.
//! Its GraphQL search was seen honouring every qualifier well past that
//! (1,450 characters), but a project's whole list hangs on this one search,
//! so each string is kept within the documented limit and the logins that
//! do not fit go into further searches.

use crate::config::is_login;

/// GitHub's documented maximum length of a search query.
pub const MAX_SEARCH_LEN: usize = 256;

/// One search to run for a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorSearch {
    pub query: String,
    /// It has `author:@me`.
    pub has_viewer: bool,
    /// It has at least one tracked contributor.
    pub has_others: bool,
}

/// The viewer's own open PRs in `owner/name`, newest update first.
pub fn viewer_search(owner: &str, name: &str) -> String {
    format!("{} author:@me", base(owner, name))
}

/// The searches covering the viewer and every contributor in `owner/name`.
/// The first always has the viewer, then as many contributors as fit; the
/// rest hold the contributors that did not fit, in order. For valid repo
/// names and logins no string goes over `MAX_SEARCH_LEN`: the longest base
/// with the viewer and the longest login is 235 characters.
///
/// Logins that are not valid GitHub usernames (a hand-edited config) are
/// skipped rather than pasted into the query, as are repeats.
pub fn author_searches(owner: &str, name: &str, contributors: &[String]) -> Vec<AuthorSearch> {
    let base = base(owner, name);
    let mut out = Vec::new();
    let mut current = AuthorSearch {
        query: viewer_search(owner, name),
        has_viewer: true,
        has_others: false,
    };
    let mut seen: Vec<String> = Vec::new();
    for login in contributors.iter().map(|c| c.trim()) {
        let key = login.to_ascii_lowercase();
        if !is_login(login) || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let piece = format!(" author:{login}");
        let empty = !current.has_viewer && !current.has_others;
        if !empty && current.query.len() + piece.len() > MAX_SEARCH_LEN {
            out.push(current);
            current = AuthorSearch {
                query: base.clone(),
                has_viewer: false,
                has_others: false,
            };
        }
        current.query.push_str(&piece);
        current.has_others = true;
    }
    out.push(current);
    out
}

/// What every search shares. Sorted so that, when a project hits the cap on
/// contributors' PRs, the ones left out are the least recently updated.
fn base(owner: &str, name: &str) -> String {
    format!("repo:{owner}/{name} is:pr is:open sort:updated-desc")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn logins(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    fn queries(searches: &[AuthorSearch]) -> Vec<&str> {
        searches.iter().map(|s| s.query.as_str()).collect()
    }

    #[test]
    fn viewer_only() {
        let s = author_searches("dashpay", "platform", &[]);
        assert_eq!(
            s,
            vec![AuthorSearch {
                query: "repo:dashpay/platform is:pr is:open sort:updated-desc author:@me".into(),
                has_viewer: true,
                has_others: false,
            }]
        );
        assert_eq!(s[0].query, viewer_search("dashpay", "platform"));
    }

    #[test]
    fn viewer_and_several_share_one_search() {
        let s = author_searches(
            "dashpay",
            "platform",
            &logins(&["shumkov", "lklimek", "PastaPastaPasta"]),
        );
        assert_eq!(
            queries(&s),
            vec![
                "repo:dashpay/platform is:pr is:open sort:updated-desc author:@me author:shumkov author:lklimek author:PastaPastaPasta"
            ]
        );
        assert!(s[0].has_viewer && s[0].has_others);
    }

    #[test]
    fn repeats_and_junk_are_skipped() {
        let s = author_searches(
            "o",
            "r",
            &logins(&[
                "bob",
                "BOB",
                " bob ",
                "x repo:evil/repo",
                "",
                "-bad",
                "carol",
            ]),
        );
        assert_eq!(
            queries(&s),
            vec!["repo:o/r is:pr is:open sort:updated-desc author:@me author:bob author:carol"]
        );
    }

    #[test]
    fn the_maximum_of_long_logins_is_split_and_nothing_is_lost() {
        // The worst case: as many contributors as a config may hold, each
        // with the longest login GitHub allows.
        let max = crate::config::MAX_CONTRIBUTORS;
        let many: Vec<String> = (0..max)
            .map(|i| format!("{i:0>2}{}", "x".repeat(37)))
            .collect();
        let s = author_searches("dashpay", "platform", &many);
        assert!(s.len() > 1);
        assert!(s.iter().all(|q| q.query.len() <= MAX_SEARCH_LEN), "{s:#?}");
        assert!(s[0].has_viewer);
        assert!(s[1..].iter().all(|q| !q.has_viewer && q.has_others));
        // Every login appears exactly once, in order.
        let found: Vec<&str> = s
            .iter()
            .flat_map(|q| q.query.split(' '))
            .filter_map(|w| w.strip_prefix("author:"))
            .filter(|w| *w != "@me")
            .collect();
        assert_eq!(found, many.iter().map(String::as_str).collect::<Vec<_>>());
        // Every search after the first starts with the same base.
        assert!(s.iter().all(|q| {
            q.query
                .starts_with("repo:dashpay/platform is:pr is:open sort:updated-desc ")
        }));
    }

    #[test]
    fn typical_logins_split_only_past_about_ten() {
        let names = [
            "shumkov",
            "lklimek",
            "HashEngineering",
            "romchornyi",
            "llbartekll",
            "bfoss765",
            "ktechmidas",
            "xdustinface",
            "DCG-Claude",
            "PastaPastaPasta",
            "Claudius-Maginificent",
        ];
        let s = author_searches("dashpay", "platform", &logins(&names[..10]));
        assert_eq!(s.len(), 1, "{:?}", queries(&s));
        assert_eq!(s[0].query.len(), 247);
        let s = author_searches("dashpay", "platform", &logins(&names));
        assert_eq!(s.len(), 2, "{:?}", queries(&s));
        assert_eq!(
            s[1].query,
            "repo:dashpay/platform is:pr is:open sort:updated-desc author:Claudius-Maginificent"
        );
        assert!(!s[1].has_viewer && s[1].has_others);
    }

    #[test]
    fn the_longest_repo_name_still_fits_one_longest_login_per_search() {
        let owner = "o".repeat(39);
        let name = "n".repeat(100);
        let long: Vec<String> = ["a", "b", "c"].iter().map(|c| c.repeat(39)).collect();
        let s = author_searches(&owner, &name, &long);
        assert_eq!(s.len(), 3, "{:?}", queries(&s));
        assert!(s.iter().all(|q| q.query.len() <= MAX_SEARCH_LEN));
        assert!(s[0].has_viewer && s[0].has_others);
        for (q, login) in s.iter().zip(&long) {
            assert!(q.query.ends_with(&format!(" author:{login}")));
        }
    }
}
