// Pure helpers for tracked contributors: how their names read in titles and
// counts, and how the "Track contributor" field matches the known list.

import type { KnownContributor } from "../types";

/** "shumkov's", and "lumens'" for a login that ends in s. */
export function possessive(login: string): string {
  return /s$/i.test(login) ? `${login}'` : `${login}'s`;
}

/** The header title of a contributor's view: "shumkov's pull requests". */
export function contributorTitle(login: string): string {
  return `${possessive(login)} pull requests`;
}

/** "1 open pull request", "3 open pull requests". */
export function openPullRequests(n: number): string {
  return `${n} open pull ${n === 1 ? "request" : "requests"}`;
}

/**
 * What the user typed into the username field, as a login to match against
 * the known list: without spaces, a leading `@` or a github.com profile link.
 * The backend does its own cleaning; this only narrows the list.
 */
export function typedLogin(input: string): string {
  return input
    .trim()
    .replace(/^(?:https?:\/\/)?(?:www\.)?github\.com\//i, "")
    .replace(/[/?#].*$/, "")
    .replace(/^@/, "");
}

/** The known people whose login contains what was typed, in their order. Empty input keeps everyone. */
export function filterKnown(people: KnownContributor[], input: string): KnownContributor[] {
  const q = typedLogin(input).toLowerCase();
  if (!q) return people;
  return people.filter((p) => p.login.toLowerCase().includes(q));
}
