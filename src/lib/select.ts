// Shared read-only helpers over a snapshot. The sidebar counts and the table
// both use these, so "Mine", "Following" and "Needs me" always agree.

import type { PullRequest, Snapshot } from "../types";

/** Which list the main pane shows. */
export type View =
  | { kind: "mine" }
  | { kind: "following" }
  | { kind: "needsMe" }
  | { kind: "project"; repo: string }
  /** The open PRs of one tracked contributor, by GitHub login. */
  | { kind: "contributor"; login: string };

/** GitHub logins are case-insensitive. */
export function sameLogin(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

export function sameView(a: View, b: View): boolean {
  if (a.kind === "project") return b.kind === "project" && a.repo === b.repo;
  if (a.kind === "contributor") return b.kind === "contributor" && sameLogin(a.login, b.login);
  return a.kind === b.kind;
}

/** In the user's own lists: theirs or followed. A tracked contributor's PR is not, unless followed. */
export function isListed(pr: PullRequest): boolean {
  return pr.isMine || pr.isFollowed;
}

/** Every PR in the snapshot, in project order. */
export function allPrs(snapshot: Snapshot | null): PullRequest[] {
  return snapshot ? snapshot.projects.flatMap((p) => p.prs) : [];
}

export function matchesView(pr: PullRequest, view: View): boolean {
  switch (view.kind) {
    case "mine":
      return pr.isMine;
    case "following":
      return pr.isFollowed;
    case "needsMe":
      return pr.needsMe;
    case "project":
      // A project lists what it always did; tracked contributors have their own views.
      return pr.repo === view.repo && isListed(pr);
    case "contributor":
      return sameLogin(pr.author, view.login);
  }
}

export function prsForView(snapshot: Snapshot | null, view: View): PullRequest[] {
  return allPrs(snapshot).filter((pr) => matchesView(pr, view));
}

export interface ViewCounts {
  mine: number;
  following: number;
  needsMe: number;
  /** Listed PRs per repo (the viewer's own plus followed). */
  byProject: Record<string, number>;
  /** Open PRs per author, keyed by lower-case login. Read it with `contributorCount`. */
  byAuthor: Record<string, number>;
}

/** How many open PRs in the snapshot `login` wrote. */
export function contributorCount(counts: ViewCounts, login: string): number {
  return counts.byAuthor[login.toLowerCase()] ?? 0;
}

export function viewCounts(snapshot: Snapshot | null): ViewCounts {
  const counts: ViewCounts = { mine: 0, following: 0, needsMe: 0, byProject: {}, byAuthor: {} };
  for (const project of snapshot?.projects ?? []) {
    counts.byProject[project.repo] = project.prs.filter(isListed).length;
    for (const pr of project.prs) {
      const author = pr.author.toLowerCase();
      counts.byAuthor[author] = (counts.byAuthor[author] ?? 0) + 1;
      if (pr.isMine) counts.mine++;
      if (pr.isFollowed) counts.following++;
      if (pr.needsMe) counts.needsMe++;
    }
  }
  return counts;
}

/** Case-insensitive match on title, number, repo and author. Empty query matches all. */
export function matchesQuery(pr: PullRequest, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  const haystack = `${pr.title} #${pr.number} ${pr.repo} ${pr.author}`.toLowerCase();
  return q.split(/\s+/).every((word) => haystack.includes(word));
}
