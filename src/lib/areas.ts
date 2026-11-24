// A project's "areas": the scope in a pull request title, as in
// "fix(sdk): ..." or "feat(platform-wallet)!: ...". The sidebar lets the user
// switch areas of a project on and off.

import type { PullRequest } from "../types";

/** Key for pull requests whose title has no scope. Same value as the store's `NO_AREA`. */
export const NO_AREA = "";

/** What the "no scope" area is called in the UI. */
export const NO_AREA_LABEL = "Other";

// type(scope)!: subject
const SCOPED_TITLE = /^\s*[A-Za-z][\w-]*\(([^)]*)\)!?:/;

/**
 * The areas a title names, lower-cased, in the order written. A title may
 * name several, separated by commas ("fix(sdk, drive): ..."). A title with no
 * scope, or an empty one, has the single area `NO_AREA`.
 */
export function titleAreas(title: string): string[] {
  const scope = SCOPED_TITLE.exec(title)?.[1];
  if (scope == null) return [NO_AREA];
  const areas = scope
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter((s) => s.length > 0);
  return areas.length ? [...new Set(areas)] : [NO_AREA];
}

export interface AreaCount {
  /** The scope, or `NO_AREA`. */
  key: string;
  /** The scope as shown, or `NO_AREA_LABEL`. */
  label: string;
  /** Pull requests naming this area. One naming two areas counts in both. */
  count: number;
}

/** The areas present among `prs`, by name, with the "no scope" area last. */
export function areaCounts(prs: readonly PullRequest[]): AreaCount[] {
  const counts = new Map<string, number>();
  for (const pr of prs) {
    for (const area of titleAreas(pr.title)) counts.set(area, (counts.get(area) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([key, count]) => ({ key, label: key === NO_AREA ? NO_AREA_LABEL : key, count }))
    .sort((a, b) => {
      if ((a.key === NO_AREA) !== (b.key === NO_AREA)) return a.key === NO_AREA ? 1 : -1;
      return a.key.localeCompare(b.key);
    });
}

/**
 * Whether a pull request is shown given the areas switched off per project.
 * It is shown while at least one of its areas is on.
 */
export function areaVisible(pr: PullRequest, hiddenAreas: Readonly<Record<string, readonly string[]>>): boolean {
  const hidden = hiddenAreas[pr.repo];
  if (!hidden || hidden.length === 0) return true;
  return titleAreas(pr.title).some((area) => !hidden.includes(area));
}
