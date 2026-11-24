// Stacked pull requests: a PR stacked on another (`PullRequest.stackedOn`)
// is listed directly under it, one step further in. Only within one list:
// when the parent is not in the same list (filtered out, in another view,
// group or sub-group), the PR keeps its own sorted place at the left edge, so
// stacking never hides a PR.

import type { PullRequest } from "../types";

/** Where a nested row sits: how deep, and the row it is stacked on. */
export interface StackPlace {
  /** How many of its ancestors are in the list: 1 directly under a top row, 2 under that. */
  depth: number;
  /** The PR it is stacked on, listed just above it or above its earlier siblings. */
  parent: PullRequest;
}

export interface Stacked {
  /** The rows in display order: each child after its parent and its parent's earlier children. */
  prs: PullRequest[];
  /** One entry per nested row, by PR id; rows at the left edge have none. */
  places: Map<string, StackPlace>;
}

const keyOf = (repo: string, number: number) => `${repo}#${number}`;

/**
 * Orders already sorted rows so each PR whose parent is in `sorted` comes
 * right after the parent (and after the parent's earlier children and their
 * descendants). Siblings keep their sorted order; a PR whose parent is not
 * listed stays where the sort put it. A cycle of `stackedOn` links (which a
 * snapshot should never hold) is cut at its member that sorts first, which
 * then sits at the left edge.
 */
export function stackPrs(sorted: readonly PullRequest[]): Stacked {
  const index = new Map<string, number>();
  sorted.forEach((pr, i) => {
    const k = keyOf(pr.repo, pr.number);
    if (!index.has(k)) index.set(k, i);
  });

  // The listed parent of each row, by row index; -1 when it has none here.
  // Older captured snapshots lack `stackedOn`: undefined means not stacked.
  const parent = sorted.map((pr, i) => {
    const on = pr.stackedOn;
    if (on == null) return -1;
    const p = index.get(keyOf(pr.repo, on));
    return p == null || p === i ? -1 : p;
  });

  // Cut cycles: follow each chain up; when it comes back to a row already on
  // the chain, detach the cycle's first row in sort order.
  for (let start = 0; start < sorted.length; start++) {
    const onChain = new Set<number>();
    let at = start;
    while (at !== -1 && !onChain.has(at)) {
      onChain.add(at);
      at = parent[at];
    }
    if (at === -1) continue;
    let first = at;
    for (let c = parent[at]; c !== at; c = parent[c]) first = Math.min(first, c);
    parent[first] = -1;
  }

  const children = sorted.map((): number[] => []);
  parent.forEach((p, i) => {
    if (p !== -1) children[p].push(i);
  });

  const prs: PullRequest[] = [];
  const places = new Map<string, StackPlace>();
  const place = (i: number, depth: number) => {
    const pr = sorted[i];
    prs.push(pr);
    if (depth > 0) places.set(pr.id, { depth, parent: sorted[parent[i]] });
    for (const c of children[i]) place(c, depth + 1);
  };
  parent.forEach((p, i) => {
    if (p === -1) place(i, 0);
  });
  return { prs, places };
}

/** "Stacked on #6132", the arrow's accessible name. */
export function stackedLabel(parent: Pick<PullRequest, "number">): string {
  return `Stacked on #${parent.number}`;
}

/** "Stacked on #6132: feat(sdk): ...", the arrow's tooltip. */
export function stackedTitle(parent: Pick<PullRequest, "number" | "title">): string {
  return `${stackedLabel(parent)}: ${parent.title}`;
}
