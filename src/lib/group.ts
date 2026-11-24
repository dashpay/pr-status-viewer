// Splits the visible rows into groups: by project or by the bot's status.
// Project groups can be split again into one sub-group per subcrate (the
// area named in the PR title, see `areas.ts`). Inside each group, or each
// sub-group when split, a stacked PR is listed under the PR it is stacked on
// when that PR is in the same list (`stack.ts`).

import type { BotReview, PullRequest, Snapshot } from "../types";
import type { GroupBy, SortBy } from "../state/store";
import { NO_AREA, NO_AREA_LABEL, titleAreas } from "./areas";
import { sortPrs } from "./sort";
import { stackPrs, type StackPlace } from "./stack";

/** Status buckets, in display order. */
export type StatusKey =
  | "reviewing"
  | "queued"
  | "changesRequested"
  | "commented"
  | "approved"
  | "draft"
  | "none";

export const STATUS_ORDER: StatusKey[] = [
  "reviewing",
  "queued",
  "changesRequested",
  "commented",
  "approved",
  "draft",
  "none",
];

/** Group header for status grouping. */
export const STATUS_LABEL: Record<StatusKey, string> = {
  reviewing: "Reviewing",
  queued: "Queued",
  changesRequested: "Changes requested",
  commented: "Comments only",
  approved: "Approved",
  draft: "Draft",
  none: "No bot review",
};

/** Lower-case phrase for a project header's summary ("4 changes requested"). */
const SUMMARY_LABEL: Record<StatusKey, string> = {
  reviewing: "reviewing",
  queued: "queued",
  changesRequested: "changes requested",
  commented: "commented",
  approved: "approved",
  draft: "draft",
  none: "no bot review",
};

export function statusKey(bot: BotReview): StatusKey {
  return bot.state === "other" ? "none" : bot.state;
}

/** "1 reviewing · 4 changes requested · 3 approved": non-zero parts in status order. */
export function statusSummary(prs: readonly PullRequest[]): string {
  const counts = new Map<StatusKey, number>();
  for (const pr of prs) {
    const k = statusKey(pr.bot);
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return STATUS_ORDER.filter((k) => counts.get(k))
    .map((k) => `${counts.get(k)} ${SUMMARY_LABEL[k]}`)
    .join(" · ");
}

export interface Group {
  /** Stable key: the repo, or `status:<key>`. */
  key: string;
  /** Header text: the repo or the status name. */
  title: string;
  /** Dim text after the title: the summary or the count. */
  meta: string;
  /** A project that could not be fetched. */
  error: string | null;
  /** Every row of the group in display order (sub-group by sub-group when split). */
  prs: PullRequest[];
  /** One per subcrate when the project group is split; null when it is not. */
  subgroups: SubGroup[] | null;
  /** The rows listed under the PR they are stacked on, by PR id; the others have none. */
  stacked: ReadonlyMap<string, StackPlace>;
}

export interface SubGroup {
  /** Stable key: `<repo>::<area>`. */
  key: string;
  /** The area key (`NO_AREA` for titles without a scope). */
  area: string;
  /** The area as the sidebar shows it: the scope, or "Other". */
  title: string;
  prs: PullRequest[];
}

/**
 * The one subcrate a pull request is listed under. A title may name several
 * areas; the list shows it under the first one written that is switched on
 * (the sidebar keeps a PR shown while any of its areas is on, so it lands
 * under one the user can see). With none switched off that is simply the
 * first area in the title.
 */
export function primaryArea(pr: PullRequest, hiddenAreas: Readonly<Record<string, readonly string[]>> = {}): string {
  const areas = titleAreas(pr.title);
  const off = hiddenAreas[pr.repo] ?? [];
  return areas.find((a) => !off.includes(a)) ?? areas[0] ?? NO_AREA;
}

/**
 * Splits already sorted rows into one sub-group per subcrate. Rows keep their
 * order inside a sub-group, and sub-groups are ordered by where their first
 * row sits, so the sort still decides what comes first.
 */
export function splitBySubcrate(
  repo: string,
  sorted: readonly PullRequest[],
  hiddenAreas: Readonly<Record<string, readonly string[]>> = {},
): SubGroup[] {
  const byArea = new Map<string, PullRequest[]>();
  for (const pr of sorted) {
    const area = primaryArea(pr, hiddenAreas);
    const list = byArea.get(area);
    if (list) list.push(pr);
    else byArea.set(area, [pr]);
  }
  // Map keeps insertion order: the order of each area's first row.
  return [...byArea.entries()].map(([area, prs]) => ({
    key: `${repo}::${area}`,
    area,
    title: area === NO_AREA ? NO_AREA_LABEL : area,
    prs,
  }));
}

export interface GroupOptions {
  /** Split each project group by subcrate. Ignored when grouping by status. */
  bySubcrate?: boolean;
  /** Areas switched off per project, to place PRs naming several areas. */
  hiddenAreas?: Readonly<Record<string, readonly string[]>>;
}

/** Groups `prs` and sorts each group. Empty groups are left out. */
export function groupPrs(
  prs: readonly PullRequest[],
  groupBy: GroupBy,
  sortBy: SortBy,
  snapshot: Snapshot | null,
  options: GroupOptions = {},
): Group[] {
  if (groupBy === "project") {
    const byRepo = new Map<string, PullRequest[]>();
    for (const pr of prs) {
      const list = byRepo.get(pr.repo);
      if (list) list.push(pr);
      else byRepo.set(pr.repo, [pr]);
    }
    const order = snapshot?.projects.map((p) => p.repo) ?? [];
    // Repos missing from the snapshot's project list (should not happen) go last.
    for (const repo of byRepo.keys()) if (!order.includes(repo)) order.push(repo);
    return order
      .filter((repo) => byRepo.has(repo))
      .map((repo) => {
        const rows = byRepo.get(repo) ?? [];
        const sorted = sortPrs(rows, sortBy);
        const stacked = new Map<string, StackPlace>();
        // Stacks form inside each list as shown: a parent in another sub-group does not pull its child over.
        const stack = (list: readonly PullRequest[]) => {
          const s = stackPrs(list);
          for (const [id, place] of s.places) stacked.set(id, place);
          return s.prs;
        };
        const subgroups = options.bySubcrate
          ? splitBySubcrate(repo, sorted, options.hiddenAreas).map((sg) => ({ ...sg, prs: stack(sg.prs) }))
          : null;
        return {
          key: repo,
          title: repo,
          meta: statusSummary(rows),
          error: snapshot?.projects.find((p) => p.repo === repo)?.error ?? null,
          prs: subgroups ? subgroups.flatMap((sg) => sg.prs) : stack(sorted),
          subgroups,
          stacked,
        };
      });
  }

  const byStatus = new Map<StatusKey, PullRequest[]>();
  for (const pr of prs) {
    const k = statusKey(pr.bot);
    const list = byStatus.get(k);
    if (list) list.push(pr);
    else byStatus.set(k, [pr]);
  }
  return STATUS_ORDER.filter((k) => byStatus.has(k)).map((k) => {
    const rows = byStatus.get(k) ?? [];
    const { prs: stackedRows, places } = stackPrs(sortPrs(rows, sortBy));
    return {
      key: `status:${k}`,
      title: STATUS_LABEL[k],
      meta: String(rows.length),
      error: null,
      prs: stackedRows,
      subgroups: null,
      stacked: places,
    };
  });
}
