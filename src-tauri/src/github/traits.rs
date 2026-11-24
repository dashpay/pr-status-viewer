//! Reads a pull request's traits from the comments a workflow posts on it.
//!
//! dashpay/platform runs two GitHub Actions workflows that post one top-level
//! comment each, as the Bot `github-actions`, and edit it in place as the PR
//! changes (`.github/workflows/grovedb-structure-preview.yml` and
//! `system-contract-preview.yml` there):
//!
//! - `<!-- grovedb-structure-preview -->`: the PR changes the described
//!   GroveDB structure. Lists `**Added** (N nodes)`, `**Changed** (...)`
//!   and `**Removed** (...)` and links the structure viewer. When the PR
//!   changes the structure file but no node, the same comment says "No node
//!   of the described GroveDB structure was added, changed or removed",
//!   which is not a trait.
//! - `<!-- system-contract-preview -->`: the PR adds or changes system
//!   contract schemas. One list item per schema file (at most 30, then
//!   "- and N more"), each "new contract", "new version", "changed" or
//!   "removed", with a compare link and/or an "open on its own" link. When
//!   the PR no longer touches a schema the comment says "This pull request
//!   no longer adds or changes a system contract's document schemas", which
//!   is not a trait.
//!
//! The wording is not ours: anything unexpected inside a marked comment
//! means no trait, never an error. Only comments by the Bot `github-actions`
//! count; anyone can type a marker. Links are opened by the app, so only
//! https links to the two viewers on `dashpay.github.io` are kept.
//!
//! Everything here is pure.

use chrono::{DateTime, Utc};
use reqwest::Url;

use crate::model::{PrTrait, TraitKind, TraitLink};

/// The workflows' account as GraphQL names it (REST says `github-actions[bot]`).
pub(crate) const AUTHOR_LOGIN: &str = "github-actions";
/// GraphQL `__typename` of a GitHub App account.
const BOT_TYPENAME: &str = "Bot";

pub(crate) const GROVEDB_MARKER: &str = "<!-- grovedb-structure-preview -->";
pub(crate) const CONTRACT_MARKER: &str = "<!-- system-contract-preview -->";

const VIEWER_HOST: &str = "dashpay.github.io";
const GROVEDB_VIEWER_PATH: &str = "/grovedb-structure-viewer/";
const CONTRACT_VIEWER_PATH: &str = "/contract-visualizer/";

/// One top-level PR comment, as much as reading traits needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraitComment {
    /// GraphQL `__typename` of the author: "Bot", "User", ...
    pub author_type: String,
    pub author_login: String,
    pub created_at: Option<DateTime<Utc>>,
    pub last_edited_at: Option<DateTime<Utc>>,
    pub body: String,
}

impl TraitComment {
    /// Posted by the workflows' account.
    fn by_workflow(&self) -> bool {
        self.author_type == BOT_TYPENAME && self.author_login.eq_ignore_ascii_case(AUTHOR_LOGIN)
    }

    /// When it last changed: edited, or else created.
    fn changed_at(&self) -> Option<DateTime<Utc>> {
        self.last_edited_at.max(self.created_at)
    }
}

/// Which marker a body starts with, if any.
fn marker_of(body: &str) -> Option<TraitKind> {
    let body = body.trim_start_matches('\u{feff}').trim_start();
    if body.starts_with(GROVEDB_MARKER) {
        Some(TraitKind::GrovedbChange)
    } else if body.starts_with(CONTRACT_MARKER) {
        Some(TraitKind::ContractChange)
    } else {
        None
    }
}

/// Whether a comment is worth keeping while paging: by the workflows'
/// account and starting with a marker. Keeps the bodies held small.
pub(crate) fn is_candidate(comment: &TraitComment) -> bool {
    comment.by_workflow() && marker_of(&comment.body).is_some()
}

/// The traits the comments report: at most one per kind, GroveDB first.
/// When several qualifying comments carry the same marker, the one changed
/// last counts (on a tie, the later one in the list).
pub(crate) fn read_traits(comments: &[TraitComment]) -> Vec<PrTrait> {
    let latest = |kind: TraitKind| {
        comments
            .iter()
            .filter(|c| c.by_workflow() && marker_of(&c.body) == Some(kind))
            .enumerate()
            .max_by_key(|(i, c)| (c.changed_at(), *i))
            .map(|(_, c)| c.body.as_str())
    };
    let grovedb = latest(TraitKind::GrovedbChange).and_then(grovedb_trait);
    let contract = latest(TraitKind::ContractChange).and_then(contract_trait);
    grovedb.into_iter().chain(contract).collect()
}

/// The GroveDB trait in a structure comment, unless it says nothing changed.
fn grovedb_trait(body: &str) -> Option<PrTrait> {
    if body.contains("No node of the described GroveDB structure was") {
        return None;
    }
    let mut counts = [("added", 0u32), ("changed", 0), ("removed", 0)];
    let mut any_heading = false;
    for line in body.lines().map(str::trim) {
        for (name, count) in &mut counts {
            if let Some(n) = heading_count(line, name) {
                *count = n;
                any_heading = true;
            }
        }
    }
    let changes = body.contains("changes the described GroveDB structure");
    if !changes && !any_heading {
        return None;
    }
    let parts: Vec<(&str, u32)> = counts.into_iter().filter(|(_, n)| *n > 0).collect();
    let summary = match parts.as_slice() {
        [] => "Structure changed".to_owned(),
        [(verb, n)] => format!("{n} {} {verb}", if *n == 1 { "node" } else { "nodes" }),
        _ => parts
            .iter()
            .map(|(verb, n)| format!("{n} {verb}"))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let links = links_in(body)
        .into_iter()
        .find_map(|(_, url)| viewer_url(url, GROVEDB_VIEWER_PATH))
        .map(|url| TraitLink {
            label: "Structure viewer".to_owned(),
            url,
        })
        .into_iter()
        .collect();
    Some(PrTrait {
        kind: TraitKind::GrovedbChange,
        summary,
        links,
    })
}

/// N from a heading line such as "**Changed** (5 nodes)", for `name`
/// "changed" (any case).
fn heading_count(line: &str, name: &str) -> Option<u32> {
    let rest = line.strip_prefix("**")?;
    let (word, rest) = rest.split_once("**")?;
    if !word.eq_ignore_ascii_case(name) {
        return None;
    }
    let inner = rest.trim().strip_prefix('(')?.split(')').next()?;
    let (n, unit) = inner.trim().split_once(' ')?;
    (unit == "node" || unit == "nodes")
        .then(|| n.parse().ok())
        .flatten()
}

/// One schema file in a contracts comment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractItem {
    /// e.g. "token-history-contract".
    contract: String,
    /// e.g. 3.
    version: u32,
    /// e.g. "token-history-contract-documents.json".
    file: String,
    status: ItemStatus,
    /// The link to open, already checked, and whether it is a comparison.
    link: Option<(String, Option<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemStatus {
    NewContract,
    NewVersion,
    Changed,
    Removed,
}

impl ItemStatus {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        // "new version. **[Compare ...", "removed ([last version](...))".
        let word = text
            .split(['.', '('])
            .next()
            .map(str::trim)
            .unwrap_or_default();
        match word {
            "new contract" => Some(Self::NewContract),
            "new version" => Some(Self::NewVersion),
            "changed" => Some(Self::Changed),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Self::NewContract => "new contract",
            Self::NewVersion => "new version",
            Self::Changed => "changed",
            Self::Removed => "removed",
        }
    }
}

/// The contracts trait in a contracts comment, unless it says no schema is
/// touched any more.
fn contract_trait(body: &str) -> Option<PrTrait> {
    if body.contains("no longer adds or changes") {
        return None;
    }
    let mut items = Vec::new();
    let mut more = 0u32;
    for line in body.lines().map(str::trim) {
        let Some(item) = line.strip_prefix("- ") else {
            continue;
        };
        if let Some(n) = item
            .strip_prefix("and ")
            .and_then(|r| r.strip_suffix(" more"))
            .and_then(|n| n.trim().parse::<u32>().ok())
        {
            more = more.saturating_add(n);
        } else if let Some(item) = contract_item(item) {
            items.push(item);
        }
    }
    let intro = body.contains("adds or changes system contract");
    if items.is_empty() && more == 0 && !intro {
        return None;
    }
    let summary = contract_summary(&items, more);
    let labels = item_labels(&items);
    let links = items
        .iter()
        .zip(labels)
        .filter_map(|(item, label)| {
            item.link.as_ref().map(|(url, _)| TraitLink {
                label,
                url: url.clone(),
            })
        })
        .collect();
    Some(PrTrait {
        kind: TraitKind::ContractChange,
        summary,
        links,
    })
}

/// One list item: "`packages/<name>-contract/schema/v<N>/<file>.json`:
/// <status>. <links>". `None` when it is not of that shape.
fn contract_item(item: &str) -> Option<ContractItem> {
    let rest = item.strip_prefix('`')?;
    let (path, rest) = rest.split_once('`')?;
    let (contract, version, file) = schema_path(path)?;
    let status = ItemStatus::parse(rest.strip_prefix(':')?)?;
    let links = links_in(rest);
    let valid =
        |text: &str, url: &str| viewer_url(url, CONTRACT_VIEWER_PATH).map(|u| (text.to_owned(), u));
    let compare = links
        .iter()
        .filter(|(text, _)| {
            text.trim_start()
                .to_ascii_lowercase()
                .starts_with("compare")
        })
        .find_map(|(text, url)| valid(text, url));
    let link = match compare {
        Some((text, url)) => Some((url, Some(text))),
        None => links
            .iter()
            .filter(|(text, _)| {
                !text
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("compare")
            })
            .find_map(|(text, url)| valid(text, url))
            .map(|(_, url)| (url, None)),
    };
    Some(ContractItem {
        contract: contract.to_owned(),
        version,
        file: file.to_owned(),
        status,
        link,
    })
}

/// `packages/<name>-contract/schema/v<N>/<file>.json` split into its
/// contract name, version and file name.
fn schema_path(path: &str) -> Option<(&str, u32, &str)> {
    let mut parts = path.split('/');
    let (Some("packages"), Some(contract), Some("schema"), Some(v), Some(file), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    let name_ok = contract.ends_with("-contract")
        && contract
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let version: u32 = v.strip_prefix('v')?.parse().ok()?;
    let file_ok = file.ends_with(".json")
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    (name_ok && file_ok).then_some((contract, version, file))
}

/// "token-history-contract v3: new version" for one file; "dashpay-contract:
/// 2 schema files changed" for several files of one contract; "3 contracts
/// changed" for several contracts.
fn contract_summary(items: &[ContractItem], more: u32) -> String {
    let mut contracts: Vec<&str> = items.iter().map(|i| i.contract.as_str()).collect();
    contracts.sort_unstable();
    contracts.dedup();
    let files = u32::try_from(items.len())
        .unwrap_or(u32::MAX)
        .saturating_add(more);
    match (items, contracts.as_slice()) {
        ([one], _) if more == 0 => {
            format!("{} v{}: {}", one.contract, one.version, one.status.words())
        }
        (_, [one]) if more == 0 => format!("{one}: {files} schema files changed"),
        (_, []) if files == 0 => "System contracts changed".to_owned(),
        (_, []) => format!("{files} schema files changed"),
        _ if more > 0 => format!("{files} schema files changed"),
        _ => format!("{} contracts changed", contracts.len()),
    }
}

/// A label per item: "token-history-contract v3 (compare with v2)",
/// "token-market-contract v1 (new contract)", ... When two items would read
/// the same (two files of one contract version), each names its file.
fn item_labels(items: &[ContractItem]) -> Vec<String> {
    let what = |item: &ContractItem| match (&item.link, item.status) {
        (Some((_, Some(text))), ItemStatus::NewVersion) => {
            // "Compare with `v2`" -> "compare with v2".
            let text = text.replace('`', "");
            let mut chars = text.trim().chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_lowercase().chain(chars).collect::<String>()
            })
        }
        (Some((_, Some(_))), ItemStatus::Changed) => "compare changes".to_owned(),
        (_, ItemStatus::Removed) => "removed, last version".to_owned(),
        (_, status) => status.words().to_owned(),
    };
    let base = |item: &ContractItem| format!("{} v{}", item.contract, item.version);
    items
        .iter()
        .map(|item| {
            let clash = items
                .iter()
                .filter(|o| o.contract == item.contract && o.version == item.version)
                .count()
                > 1;
            if clash {
                format!("{} {} ({})", base(item), item.file, what(item))
            } else {
                format!("{} ({})", base(item), what(item))
            }
        })
        .collect()
}

/// Markdown links `[text](url)` in `text`, in order. Nested brackets in the
/// text are not supported; the workflows do not write them.
fn links_in(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(mid) = rest.find("](") {
        let label_start = rest[..mid].rfind('[').map_or(mid, |i| i + 1);
        let label = &rest[label_start..mid];
        let after = &rest[mid + 2..];
        let end = after
            .find(|c: char| c == ')' || c.is_whitespace())
            .unwrap_or(after.len());
        out.push((label, &after[..end]));
        rest = &after[end..];
    }
    out
}

/// `raw` as a URL the app may open: https, exactly `dashpay.github.io`, no
/// user info, no port, path under `path_prefix`. Returns it normalized.
fn viewer_url(raw: &str, path_prefix: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    let ok = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str() == Some(VIEWER_HOST)
        && url.path().starts_with(path_prefix);
    ok.then(|| url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/github/traits/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn at(s: &str) -> Option<DateTime<Utc>> {
        Some(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    fn comment(body: &str) -> TraitComment {
        TraitComment {
            author_type: "Bot".into(),
            author_login: "github-actions".into(),
            created_at: at("2026-11-01T10:00:00Z"),
            last_edited_at: None,
            body: body.into(),
        }
    }

    fn traits_of(bodies: &[&str]) -> Vec<PrTrait> {
        let comments: Vec<TraitComment> = bodies.iter().map(|b| comment(b)).collect();
        read_traits(&comments)
    }

    fn one(body: &str) -> Option<PrTrait> {
        let mut t = traits_of(&[body]);
        assert!(t.len() <= 1);
        t.pop()
    }

    fn link(label: &str, url: &str) -> TraitLink {
        TraitLink {
            label: label.into(),
            url: url.into(),
        }
    }

    const GROVEDB_URL_5987: &str = "https://dashpay.github.io/grovedb-structure-viewer/?base=083c865a229a40a722af3998ed691fb45f97a9d8&head=dashpay%2Fplatform%405acaa40705ff964b7909888eb9442e2ff13c47b8";

    /// A structure comment as the workflow writes it, with `headings`.
    fn grovedb(headings: &str) -> String {
        format!(
            "<!-- grovedb-structure-preview -->\n🌳 **GroveDB structure**\n\nThis pull request changes the described GroveDB structure. **[Open it in the structure viewer](https://dashpay.github.io/grovedb-structure-viewer/?base=aa&head=dashpay%2Fplatform%40bb)**: new nodes glow, removed ones stay as ghosts, and the tour walks through each change.\n\n{headings}_Compared `aa` with `bb`. Updated at 2026-11-19T06:18:26.120Z_"
        )
    }

    #[test]
    fn real_grovedb_comments() {
        // dashpay/platform#5987: 5 nodes changed.
        assert_eq!(
            one(&fixture("grovedb_5987.md")),
            Some(PrTrait {
                kind: TraitKind::GrovedbChange,
                summary: "5 nodes changed".into(),
                links: vec![link("Structure viewer", GROVEDB_URL_5987)],
            })
        );
        // dashpay/platform#4847: all three headings.
        let t = one(&fixture("grovedb_4847.md")).unwrap();
        assert_eq!(t.summary, "1 added, 3 changed, 2 removed");
        assert_eq!(t.links.len(), 1);
        // dashpay/platform#4830: added and removed.
        assert_eq!(
            one(&fixture("grovedb_4830.md")).unwrap().summary,
            "6 added, 1 removed"
        );
        // dashpay/platform#4848: 35 changed, listed up to 30 then "- and 5 more".
        assert_eq!(
            one(&fixture("grovedb_4848.md")).unwrap().summary,
            "35 nodes changed"
        );
        // dashpay/platform#5975: the structure file changed, no node did.
        assert_eq!(one(&fixture("grovedb_5975.md")), None);
    }

    #[test]
    fn every_grovedb_heading_combination() {
        let h = |name: &str, n: u32| {
            let unit = if n == 1 { "node" } else { "nodes" };
            format!("**{name}** ({n} {unit})\n- `a.b`\n\n")
        };
        let cases = [
            (h("Added", 2), "2 nodes added"),
            (h("Added", 1), "1 node added"),
            (h("Changed", 5), "5 nodes changed"),
            (h("Removed", 1), "1 node removed"),
            (h("Added", 2) + &h("Changed", 5), "2 added, 5 changed"),
            (h("Added", 2) + &h("Removed", 1), "2 added, 1 removed"),
            (h("Changed", 5) + &h("Removed", 3), "5 changed, 3 removed"),
            (
                h("Added", 2) + &h("Changed", 5) + &h("Removed", 1),
                "2 added, 5 changed, 1 removed",
            ),
            // The sentence without any heading it can count.
            (String::new(), "Structure changed"),
            // A heading of unknown shape is not counted.
            ("**Changed** (lots)\n\n".to_owned(), "Structure changed"),
        ];
        for (headings, summary) in cases {
            let t = one(&grovedb(&headings)).unwrap_or_else(|| panic!("{headings}"));
            assert_eq!(t.summary, summary, "{headings}");
            assert_eq!(t.kind, TraitKind::GrovedbChange);
            assert_eq!(
                t.links,
                vec![link(
                    "Structure viewer",
                    "https://dashpay.github.io/grovedb-structure-viewer/?base=aa&head=dashpay%2Fplatform%40bb"
                )]
            );
        }
    }

    #[test]
    fn unknown_grovedb_wording_is_no_trait() {
        assert_eq!(
            one("<!-- grovedb-structure-preview -->\nSomething else entirely."),
            None
        );
        assert_eq!(one("<!-- grovedb-structure-preview -->"), None);
        // Headings alone still say what changed.
        assert_eq!(
            one("<!-- grovedb-structure-preview -->\n**Removed** (4 nodes)\n")
                .unwrap()
                .summary,
            "4 nodes removed"
        );
    }

    #[test]
    fn real_contract_comments() {
        let compare_5531 = "https://dashpay.github.io/contract-visualizer/?base=https%3A%2F%2Fgithub.com%2Fdashpay%2Fplatform%2Fblob%2Fab7f2d1acb37b2af2262d79cf8664933c39c8389%2Fpackages%2Ftoken-history-contract%2Fschema%2Fv2%2Ftoken-history-contract-documents.json&head=https%3A%2F%2Fgithub.com%2Fdashpay%2Fplatform%2Fblob%2F0206ba76f60a1e84da193fb8af0b1576b69cb173%2Fpackages%2Ftoken-history-contract%2Fschema%2Fv3%2Ftoken-history-contract-documents.json";
        // dashpay/platform#5531: a new version, compared with v2.
        assert_eq!(
            one(&fixture("contract_5531.md")),
            Some(PrTrait {
                kind: TraitKind::ContractChange,
                summary: "token-history-contract v3: new version".into(),
                links: vec![link(
                    "token-history-contract v3 (compare with v2)",
                    compare_5531
                )],
            })
        );
        // dashpay/platform#5526: a new contract, with only the "open" link.
        let t = one(&fixture("contract_5526.md")).unwrap();
        assert_eq!(t.summary, "token-market-contract v1: new contract");
        assert_eq!(
            t.links,
            vec![link(
                "token-market-contract v1 (new contract)",
                "https://dashpay.github.io/contract-visualizer/?url=https%3A%2F%2Fgithub.com%2Fdashpay%2Fplatform%2Fblob%2F9931a98cd4cbb60a00158ed0c01eac9b124ee8f2%2Fpackages%2Ftoken-market-contract%2Fschema%2Fv1%2Ftoken-market-contract-documents.json"
            )]
        );
        // dashpay/platform#5804: a changed file, compared in place.
        let t = one(&fixture("contract_5804.md")).unwrap();
        assert_eq!(t.summary, "dashpay-contract v3: changed");
        assert_eq!(t.links.len(), 1);
        assert_eq!(t.links[0].label, "dashpay-contract v3 (compare changes)");
        assert!(t.links[0].url.contains("?base="));
    }

    const C_INTRO: &str = "<!-- system-contract-preview -->\n📜 **System contracts**\n\nThis pull request adds or changes system contract document schemas. The contract viewer diagrams each one, and a comparison marks what is added, changed and removed, with the update rule each change meets.\n\n";
    const C_END: &str = "\n\n_Compared `aa` with `bb`. Updated at 2026-11-01T09:14:53.561Z_";
    const V: &str = "https://dashpay.github.io/contract-visualizer/";

    fn contracts(rows: &[String]) -> String {
        format!("{C_INTRO}{}{C_END}", rows.join("\n"))
    }

    fn row(contract: &str, v: u32, status: &str, links: &str) -> String {
        format!("- `packages/{contract}/schema/v{v}/{contract}-documents.json`: {status}. {links}")
    }

    #[test]
    fn several_contracts_and_each_link_kind() {
        let body = contracts(&[
            row(
                "dpns-contract",
                3,
                "new version",
                &format!(
                    "**[Compare with `v2`]({V}?base=a&head=b)** · [open on its own]({V}?url=b)"
                ),
            ),
            row(
                "dashpay-contract",
                3,
                "changed",
                &format!(
                    "**[Compare in the contract viewer]({V}?base=c&head=d)** · [open on its own]({V}?url=d)"
                ),
            ),
            row(
                "wallet-utils-contract",
                1,
                "new contract",
                &format!("**[Open it in the contract viewer]({V}?url=e)**"),
            ),
            format!(
                "- `packages/withdrawals-contract/schema/v1/withdrawals-contract-documents.json`: removed ([last version]({V}?url=f))"
            ),
        ]);
        let t = one(&body).unwrap();
        assert_eq!(t.summary, "4 contracts changed");
        assert_eq!(
            t.links,
            vec![
                link(
                    "dpns-contract v3 (compare with v2)",
                    &format!("{V}?base=a&head=b")
                ),
                link(
                    "dashpay-contract v3 (compare changes)",
                    &format!("{V}?base=c&head=d")
                ),
                link(
                    "wallet-utils-contract v1 (new contract)",
                    &format!("{V}?url=e")
                ),
                link(
                    "withdrawals-contract v1 (removed, last version)",
                    &format!("{V}?url=f")
                ),
            ]
        );
    }

    #[test]
    fn a_compare_link_on_another_host_falls_back_to_open_on_its_own() {
        let body = contracts(&[row(
            "dpns-contract",
            3,
            "new version",
            &format!(
                "**[Compare with `v2`](https://evil.example/contract-visualizer/?x)** · [open on its own]({V}?url=b)"
            ),
        )]);
        assert_eq!(
            one(&body).unwrap().links,
            vec![link(
                "dpns-contract v3 (new version)",
                &format!("{V}?url=b")
            )]
        );
        // Only "open on its own" was written.
        let body = contracts(&[row(
            "dashpay-contract",
            2,
            "changed",
            &format!("[open on its own]({V}?url=z)"),
        )]);
        assert_eq!(
            one(&body).unwrap().links,
            vec![link("dashpay-contract v2 (changed)", &format!("{V}?url=z"))]
        );
    }

    #[test]
    fn one_contract_several_files_and_more() {
        let body = contracts(&[
            format!(
                "- `packages/dashpay-contract/schema/v3/dashpay.schema.json`: changed. **[Compare in the contract viewer]({V}?base=1)**"
            ),
            format!(
                "- `packages/dashpay-contract/schema/v3/extra.schema.json`: changed. **[Compare in the contract viewer]({V}?base=2)**"
            ),
        ]);
        let t = one(&body).unwrap();
        assert_eq!(t.summary, "dashpay-contract: 2 schema files changed");
        assert_eq!(
            t.links.iter().map(|l| l.label.as_str()).collect::<Vec<_>>(),
            vec![
                "dashpay-contract v3 dashpay.schema.json (compare changes)",
                "dashpay-contract v3 extra.schema.json (compare changes)",
            ]
        );
        let body = contracts(&[
            row(
                "dpns-contract",
                3,
                "changed",
                &format!("**[Compare in the contract viewer]({V}?base=1)**"),
            ),
            "- and 31 more".to_owned(),
        ]);
        let t = one(&body).unwrap();
        assert_eq!(t.summary, "32 schema files changed");
        assert_eq!(t.links.len(), 1);
    }

    #[test]
    fn contracts_no_longer_touched_is_no_trait() {
        // The workflow's wording once a PR stops touching a schema (no list).
        let body = "<!-- system-contract-preview -->\n📜 **System contracts**\n\nThis pull request no longer adds or changes a system contract's document schemas.\n\n\n_Compared `aa` with `bb`. Updated at 2026-11-01T09:14:53.561Z_";
        assert_eq!(one(body), None);
        // Unknown wording, no list it can read.
        assert_eq!(one("<!-- system-contract-preview -->\nHello."), None);
    }

    #[test]
    fn unreadable_contract_items_keep_the_trait_without_links() {
        let body = contracts(&["- `somewhere/else.json`: changed. [x](https://dashpay.github.io/contract-visualizer/?u)".to_owned()]);
        assert_eq!(
            one(&body),
            Some(PrTrait {
                kind: TraitKind::ContractChange,
                summary: "System contracts changed".into(),
                links: vec![],
            })
        );
    }

    #[test]
    fn only_the_workflow_bot_counts() {
        let body = fixture("grovedb_5987.md");
        let person = TraitComment {
            author_type: "User".into(),
            author_login: "github-actions".into(),
            ..comment(&body)
        };
        let other_bot = TraitComment {
            author_login: "coderabbitai".into(),
            ..comment(&body)
        };
        let rest_name = TraitComment {
            author_login: "someone".into(),
            author_type: "User".into(),
            ..comment(&body)
        };
        for c in [&person, &other_bot, &rest_name] {
            assert!(!is_candidate(c), "{c:?}");
            assert_eq!(read_traits(std::slice::from_ref(c)), vec![]);
        }
        let real = comment(&body);
        assert!(is_candidate(&real));
        let upper = TraitComment {
            author_login: "GitHub-Actions".into(),
            ..comment(&body)
        };
        assert_eq!(read_traits(&[upper]).len(), 1);
        // The marker must open the comment, not sit inside a quote of it.
        let quoted = comment(&format!("> {body}"));
        assert!(!is_candidate(&quoted));
        assert_eq!(read_traits(&[quoted]), vec![]);
    }

    #[test]
    fn links_must_be_the_viewers_on_dashpay_github_io() {
        let with = |url: &str| {
            one(&format!(
                "<!-- grovedb-structure-preview -->\nThis pull request changes the described GroveDB structure. **[Open it in the structure viewer]({url})**\n**Changed** (1 node)\n"
            ))
            .unwrap()
            .links
        };
        let good = "https://dashpay.github.io/grovedb-structure-viewer/?base=a&head=b";
        assert_eq!(with(good), vec![link("Structure viewer", good)]);
        for bad in [
            "http://dashpay.github.io/grovedb-structure-viewer/?base=a",
            "https://dashpay.github.io.evil.example/grovedb-structure-viewer/",
            "https://evil.example/grovedb-structure-viewer/",
            "https://evil-dashpay.github.io/grovedb-structure-viewer/",
            "https://dashpay.github.io@evil.example/grovedb-structure-viewer/",
            "https://user:pw@dashpay.github.io/grovedb-structure-viewer/",
            "https://user@dashpay.github.io/grovedb-structure-viewer/",
            "https://dashpay.github.io:8443/grovedb-structure-viewer/",
            "https://dashpay.github.io/other-app/",
            "https://dashpay.github.io/grovedb-structure-viewer-evil/",
            "https://dashpay.github.io/grovedb-structure-viewer/../evil/",
            "https://dashpay.github.io/contract-visualizer/?url=x",
            "javascript:alert(1)",
            "//dashpay.github.io/grovedb-structure-viewer/",
            "",
        ] {
            assert_eq!(with(bad), vec![], "{bad}");
        }
        // The contract viewer is for contracts only, and vice versa.
        let body = contracts(&[row(
            "dpns-contract",
            1,
            "new contract",
            "**[Open it](https://dashpay.github.io/grovedb-structure-viewer/?x)**",
        )]);
        assert_eq!(one(&body).unwrap().links, vec![]);
        // Upper-case host is the same host.
        assert_eq!(
            with("https://DASHPAY.github.io/grovedb-structure-viewer/?q"),
            vec![link(
                "Structure viewer",
                "https://dashpay.github.io/grovedb-structure-viewer/?q"
            )]
        );
    }

    #[test]
    fn the_latest_comment_with_a_marker_wins() {
        let changed = grovedb("**Changed** (5 nodes)\n\n");
        let added = grovedb("**Added** (2 nodes)\n\n");
        let older = TraitComment {
            created_at: at("2026-11-01T10:00:00Z"),
            last_edited_at: at("2026-11-05T10:00:00Z"),
            ..comment(&changed)
        };
        let newer = TraitComment {
            created_at: at("2026-11-03T10:00:00Z"),
            last_edited_at: None,
            ..comment(&added)
        };
        // Edited on the 5th beats created on the 3rd, in either order.
        assert_eq!(
            read_traits(&[older.clone(), newer.clone()])[0].summary,
            "5 nodes changed"
        );
        assert_eq!(
            read_traits(&[newer.clone(), older.clone()])[0].summary,
            "5 nodes changed"
        );
        // The latest says nothing changed: no trait, even though an older one did.
        let nothing = TraitComment {
            created_at: at("2026-11-09T10:00:00Z"),
            ..comment(&fixture("grovedb_5975.md"))
        };
        assert_eq!(read_traits(&[older.clone(), nothing]), vec![]);
        // A person's newer marked comment does not hide the workflow's.
        let person = TraitComment {
            author_type: "User".into(),
            author_login: "alice".into(),
            created_at: at("2026-11-20T10:00:00Z"),
            ..comment(&added)
        };
        assert_eq!(read_traits(&[older, person])[0].summary, "5 nodes changed");
    }

    #[test]
    fn grovedb_comes_first_and_nothing_means_nothing() {
        assert_eq!(read_traits(&[]), vec![]);
        let t = traits_of(&[
            &fixture("contract_5531.md"),
            "Looks good to me",
            &fixture("grovedb_5987.md"),
        ]);
        assert_eq!(
            t.iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![TraitKind::GrovedbChange, TraitKind::ContractChange]
        );
        // Other workflow comments are not traits.
        assert_eq!(
            traits_of(&[
                "<!-- book-preview -->\n📖 **Book Preview** built successfully.",
                "<!-- platform-pr-review-state-v1 {\"admitted_at\":null} -->"
            ]),
            vec![]
        );
    }

    #[test]
    fn markdown_links_are_found_in_order() {
        assert_eq!(
            links_in("a **[One](https://x/1)** · [two](https://x/2) [bare] (no)"),
            vec![("One", "https://x/1"), ("two", "https://x/2")]
        );
        assert_eq!(links_in("broken ](https://x/3"), vec![("", "https://x/3")]);
        assert_eq!(links_in("none"), Vec::<(&str, &str)>::new());
    }

    #[test]
    fn windows_line_endings_read_the_same() {
        let body =
            grovedb("**Added** (2 nodes)\n- `a`\n\n**Removed** (1 node)\n\n").replace('\n', "\r\n");
        assert_eq!(one(&body).unwrap().summary, "2 added, 1 removed");
    }
}
