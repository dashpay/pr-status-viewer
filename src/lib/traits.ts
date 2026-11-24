// Traits: notable things a pull request does (changes the GroveDB structure,
// changes a system contract), as reported by workflow comments. Names, the
// order they are shown in, and the text for their buttons and tooltips.

import type { PrTrait, PullRequest, TraitKind } from "../types";

/** Display order. A kind not listed here is not shown. */
export const TRAIT_KINDS: readonly TraitKind[] = ["grovedbChange", "contractChange"];

export const TRAIT_NAME: Record<TraitKind, string> = {
  grovedbChange: "GroveDB change",
  contractChange: "Contract change",
};

/** What the trait's links open, for "Open the …". */
const VIEWER_NAME: Record<TraitKind, string> = {
  grovedbChange: "structure viewer",
  contractChange: "contract viewer",
};

/**
 * The pull request's traits that the UI knows how to show, in display order.
 * A snapshot from a backend older than traits has no `traits` at all.
 */
export function traitsOf(pr: PullRequest): PrTrait[] {
  const traits = (pr as { traits?: PrTrait[] | null }).traits;
  if (!Array.isArray(traits) || traits.length === 0) return [];
  return traits
    .filter((t) => t != null && TRAIT_KINDS.includes(t.kind))
    .sort((a, b) => TRAIT_KINDS.indexOf(a.kind) - TRAIT_KINDS.indexOf(b.kind));
}

/** The most traits any of these pull requests has: how many icon slots the column needs. */
export function traitSlots(prs: PullRequest[]): number {
  let most = 0;
  for (const pr of prs) most = Math.max(most, traitsOf(pr).length);
  return most;
}

/** The trait's links, without any that have no URL. */
export function traitLinks(t: PrTrait): PrTrait["links"] {
  return Array.isArray(t.links) ? t.links.filter((l) => l != null && typeof l.url === "string" && l.url !== "") : [];
}

/** "GroveDB change: 5 nodes changed", or just the name when there is no summary. */
export function traitDescription(t: PrTrait): string {
  const summary = t.summary?.trim();
  return summary ? `${TRAIT_NAME[t.kind]}: ${summary}` : TRAIT_NAME[t.kind];
}

/** What clicking the trait's icon does, or null when it does nothing. */
export function traitAction(t: PrTrait): string | null {
  const n = traitLinks(t).length;
  if (n === 0) return null;
  if (n === 1) return `Open the ${VIEWER_NAME[t.kind]}`;
  return `Choose one of ${n} links to open`;
}

/** The icon's accessible name and tooltip: "GroveDB change: 5 nodes changed. Open the structure viewer". */
export function traitLabel(t: PrTrait): string {
  const action = traitAction(t);
  return action ? `${traitDescription(t)}. ${action}` : traitDescription(t);
}
