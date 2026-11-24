// The main pane: toolbar (filter, group, sort), the filter line (drafts,
// created recently and milestone switches), and the grouped list of PR rows
// for the current view, with keyboard navigation. A row is listed when the
// text filter, the drafts switch, the created recently switch, the milestone
// switches and the sidebar's area switches all let it through. With "By
// subcrate" ticked, each project group is split into one sub-group per
// subcrate; keyboard order follows the rows as shown, stacked PRs under the
// PR they are stacked on included.

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useApp } from "../../state/store";
import { matchesQuery, prsForView, type View } from "../../lib/select";
import { groupPrs, type Group, type SubGroup } from "../../lib/group";
import { milestoneChips, milestoneShown, showMilestoneRow } from "../../lib/milestones";
import {
  applySwitches,
  draftCount,
  draftShown,
  hiddenNote,
  recentCount,
  recentShown,
  reposOf,
  showDraftsSwitch,
  showRecentSwitch,
} from "../../lib/filters";
import { areaVisible, NO_AREA } from "../../lib/areas";
import type { PullRequest } from "../../types";
import { needsFastTick } from "../../lib/format";
import { useNow } from "../../lib/useNow";
import { focusIsBusy, isDialogActive, isOwnActivationControl } from "../../lib/keys";
import { traitSlots } from "../../lib/traits";
import { Toolbar } from "./Toolbar";
import { MilestoneBar } from "./MilestoneBar";
import { DraftsSwitch, FilterBar, RecentSwitch } from "./FilterBar";
import { PrRow, domId } from "./PrRow";
import "./PrTable.css";

export function PrTable() {
  const snapshot = useApp((s) => s.snapshot);
  const view = useApp((s) => s.view);
  const groupBy = useApp((s) => s.groupBy);
  const sortBy = useApp((s) => s.sortBy);
  const bySubcrate = useApp((s) => s.bySubcrate);
  const query = useApp((s) => s.query);
  const hiddenMilestones = useApp((s) => s.hiddenMilestones);
  const showAllMilestones = useApp((s) => s.showAllMilestones);
  const hiddenAreas = useApp((s) => s.hiddenAreas);
  const showAllAreas = useApp((s) => s.showAllAreas);
  const showDrafts = useApp((s) => s.showDrafts);
  const setShowDrafts = useApp((s) => s.setShowDrafts);
  const recentOnly = useApp((s) => s.recentOnly);
  const setRecentOnly = useApp((s) => s.setRecentOnly);
  const expandedId = useApp((s) => s.expandedId);
  const focusedId = useApp((s) => s.focusedId);
  const paletteOpen = useApp((s) => s.paletteOpen);
  const setQuery = useApp((s) => s.setQuery);
  const toggleExpanded = useApp((s) => s.toggleExpanded);
  const setFocused = useApp((s) => s.setFocused);
  const openUrl = useApp((s) => s.openUrl);

  const filterRef = useRef<HTMLInputElement>(null);
  const rowEls = useRef(new Map<string, HTMLElement>());

  // Tick every second only while a visible countdown is close, or an open
  // running review shows its live elapsed time (decided below, once the
  // listed rows are known).
  const [fast, setFast] = useState(false);
  const now = useNow(fast);
  // "Created recently" is judged against a clock that moves once a minute,
  // so the list re-evaluates as time passes without regrouping every second.
  const ageClock = Math.floor(now / 60_000) * 60_000;

  const inView = useMemo(() => prsForView(snapshot, view), [snapshot, view]);
  // Chips and the switch counts come from the whole view, so they stay put
  // while filtering (hiding drafts does not make a milestone chip vanish).
  const chips = useMemo(() => milestoneChips(inView), [inView]);
  const drafts = useMemo(() => draftCount(inView), [inView]);
  const recent = useMemo(() => recentCount(inView, ageClock), [inView, ageClock]);
  const msRow = showMilestoneRow(chips, hiddenMilestones);
  const draftsRow = showDraftsSwitch(drafts, showDrafts);
  const recentRow = showRecentSwitch(recent, recentOnly);
  const matched = useMemo(() => inView.filter((pr) => matchesQuery(pr, query)), [inView, query]);
  // The clock only matters while the switch is on.
  const switchClock = recentOnly ? ageClock : 0;
  const switched = useMemo(
    () => applySwitches(matched, hiddenMilestones, hiddenAreas, showDrafts, recentOnly, switchClock),
    [matched, hiddenMilestones, hiddenAreas, showDrafts, recentOnly, switchClock],
  );
  const visible = switched.visible;
  const note = hiddenNote({
    byDraft: switched.byDraft.length,
    byAge: switched.byAge.length,
    byMilestone: switched.byMilestone.length,
    byArea: switched.byArea.length,
  });
  const groups = useMemo(
    () => groupPrs(visible, groupBy, sortBy, snapshot, { bySubcrate, hiddenAreas }),
    [visible, groupBy, sortBy, snapshot, bySubcrate, hiddenAreas],
  );
  const order = useMemo(() => groups.flatMap((g) => g.prs), [groups]);
  // Icon slots for the traits column: the most traits any listed row has.
  const slots = useMemo(() => traitSlots(order), [order]);

  const expandedPr = expandedId ? order.find((p) => p.id === expandedId) : undefined;
  const wantFast =
    order.some((pr) => needsFastTick(pr.bot, now)) ||
    (expandedPr?.bot.state === "reviewing" && expandedPr.bot.startedAt != null);
  useEffect(() => {
    if (wantFast !== fast) setFast(wantFast);
  }, [wantFast, fast]);

  const rowRef = useCallback((id: string, el: HTMLElement | null) => {
    if (el) rowEls.current.set(id, el);
    else rowEls.current.delete(id);
  }, []);

  // Bring the focused row into view and give it DOM focus, unless the user
  // is typing somewhere, a menu or dialog is open, or focus is already on a
  // control inside the row (a trait icon).
  useEffect(() => {
    if (!focusedId) return;
    const el = rowEls.current.get(focusedId);
    if (!el) return;
    el.scrollIntoView?.({ block: "nearest" });
    const active = document.activeElement;
    if (active !== el && !el.contains(active) && !focusIsBusy(active) && !isDialogActive()) el.focus({ preventScroll: true });
  }, [focusedId, expandedId]);

  const focusList = useCallback(() => {
    const target = focusedId && order.some((p) => p.id === focusedId) ? focusedId : order[0]?.id;
    if (!target) {
      filterRef.current?.blur();
      return;
    }
    if (target !== focusedId) setFocused(target);
    rowEls.current.get(target)?.focus({ preventScroll: true });
  }, [focusedId, order, setFocused]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || paletteOpen) return;
      const active = document.activeElement;
      if (focusIsBusy(active) || isDialogActive()) return;

      const current = focusedId ? order.findIndex((p) => p.id === focusedId) : -1;
      const focusedPr = current >= 0 ? order[current] : undefined;
      const move = (delta: number) => {
        if (order.length === 0) return;
        const next = current < 0 ? (delta > 0 ? 0 : order.length - 1) : Math.min(order.length - 1, Math.max(0, current + delta));
        const id = order[next].id;
        setFocused(id);
        const el = rowEls.current.get(id);
        el?.focus({ preventScroll: true });
        el?.scrollIntoView?.({ block: "nearest" });
      };

      if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.altKey) {
        if (focusedPr) {
          e.preventDefault();
          void openUrl(focusedPr.url);
        }
        return;
      }
      if (e.metaKey || e.ctrlKey || e.altKey) return;

      switch (e.key) {
        case "ArrowDown":
        case "j":
          e.preventDefault();
          move(1);
          return;
        case "ArrowUp":
        case "k":
          e.preventDefault();
          move(-1);
          return;
        case "Enter":
        case " ":
          if (!focusedPr || isOwnActivationControl(active)) return;
          e.preventDefault();
          toggleExpanded(focusedPr.id);
          return;
        case "o":
          if (!focusedPr) return;
          e.preventDefault();
          void openUrl(focusedPr.url);
          return;
        case "/":
          e.preventDefault();
          filterRef.current?.focus();
          filterRef.current?.select();
          return;
        case "Escape":
          if (expandedId) {
            e.preventDefault();
            toggleExpanded(expandedId);
          }
          return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [order, focusedId, expandedId, paletteOpen, setFocused, toggleExpanded, openUrl]);

  const onFilterEscape = useCallback(() => {
    setQuery("");
    focusList();
  }, [setQuery, focusList]);

  // The row that takes Tab focus: the focused one if visible, else the first.
  const tabStopId = focusedId && order.some((p) => p.id === focusedId) ? focusedId : order[0]?.id;

  const renderRow = (g: Group) => (pr: PullRequest) => (
    <PrRow
      key={pr.id}
      pr={pr}
      now={now}
      showRepo={groupBy === "status"}
      expanded={pr.id === expandedId}
      focused={pr.id === focusedId}
      tabStop={pr.id === tabStopId}
      stack={g.stacked.get(pr.id)}
      onToggle={toggleExpanded}
      onFocusRow={setFocused}
      rowRef={rowRef}
    />
  );

  return (
    <div className="pr-table">
      <Toolbar inputRef={filterRef} onEscape={onFilterEscape} />
      {(msRow || draftsRow || recentRow) && (
        <FilterBar>
          {draftsRow && <DraftsSwitch count={drafts} />}
          {recentRow && <RecentSwitch count={recent} />}
          {msRow && <MilestoneBar chips={chips} />}
        </FilterBar>
      )}
      <div
        className={slots > 0 ? "pr-table__scroll has-traits" : "pr-table__scroll"}
        style={slots > 0 ? ({ "--trait-slots": slots } as CSSProperties) : undefined}
      >
        {snapshot &&
          (order.length === 0 ? (
            inView.length === 0 ? (
              <EmptyView view={view} projectError={projectError(snapshot, view)} />
            ) : matched.length === 0 ? (
              <NoMatch
                query={query}
                onClear={() => {
                  setQuery("");
                  filterRef.current?.focus();
                }}
              />
            ) : (
              <AllHidden
                count={matched.length}
                filtered={query.trim() !== ""}
                byDraft={switched.byDraft.length}
                byAge={switched.byAge.length}
                byMilestone={switched.byMilestone.length}
                byArea={switched.byArea.length}
                draftsHide={matched.some((pr) => !draftShown(pr, showDrafts))}
                recentHides={matched.some((pr) => !recentShown(pr, recentOnly, switchClock))}
                milestonesHide={matched.some((pr) => !milestoneShown(pr, hiddenMilestones))}
                areaRepos={reposOf(matched.filter((pr) => !areaVisible(pr, hiddenAreas)))}
                onShowDrafts={() => setShowDrafts(true)}
                onShowOlder={() => setRecentOnly(false)}
                onShowAllMilestones={showAllMilestones}
                onShowAllAreas={(repos) => repos.forEach(showAllAreas)}
              />
            )
          ) : (
            <>
              {groups.map((g) => (
                <section key={g.key} className="pr-group" aria-labelledby={`pr-group-${domId(g.key)}`}>
                  <h2 className="pr-group__head" id={`pr-group-${domId(g.key)}`}>
                    <span className="pr-group__title">{g.title}</span>
                    {g.meta && <span className="pr-group__meta dim">{g.meta}</span>}
                    {g.error && (
                      <span className="pr-group__error" title={g.error}>
                        ⚠ Could not refresh: {g.error}
                      </span>
                    )}
                  </h2>
                  {g.subgroups
                    ? g.subgroups.map((sg) => (
                        <SubGroupSection key={sg.key} subgroup={sg}>
                          {sg.prs.map(renderRow(g))}
                        </SubGroupSection>
                      ))
                    : g.prs.map(renderRow(g))}
                </section>
              ))}
              {note && (
                <p className="pr-hidden-note dim" role="status">
                  {note}
                </p>
              )}
            </>
          ))}
      </div>
    </div>
  );
}

const prsWord = (n: number) => (n === 1 ? "pull request" : "pull requests");

/** One subcrate inside a project group: a small heading ("sdk, 3 pull requests") and its rows. */
function SubGroupSection({ subgroup, children }: { subgroup: SubGroup; children: ReactNode }) {
  const id = `pr-subgroup-${domId(subgroup.key)}`;
  const n = subgroup.prs.length;
  const cls = subgroup.area === NO_AREA ? "pr-subgroup pr-subgroup--none" : "pr-subgroup";
  return (
    <section className={cls} aria-labelledby={id}>
      <h3 className="pr-subgroup__head" id={id} aria-label={`${subgroup.title}, ${n} ${prsWord(n)}`}>
        <span className="pr-subgroup__title">{subgroup.title}</span>
        <span className="pr-subgroup__count dim">{n}</span>
      </h3>
      {children}
    </section>
  );
}

function projectError(snapshot: { projects: { repo: string; error: string | null }[] }, view: View): string | null {
  if (view.kind !== "project") return null;
  return snapshot.projects.find((p) => p.repo === view.repo)?.error ?? null;
}

function emptyGuidance(view: View): string {
  switch (view.kind) {
    case "mine":
      return "Your open pull requests in the projects you track show up here.";
    case "following":
      return "To follow someone else's pull request, open a project in the sidebar and choose one to follow.";
    case "needsMe":
      return "Nothing is waiting on you.";
    case "project":
      return `Your open pull requests in ${view.repo}, and the ones you follow there, show up here.`;
    case "contributor":
      return `${view.login} has no open pull requests in the projects you track.`;
  }
}

function EmptyView({ view, projectError }: { view: View; projectError: string | null }) {
  return (
    <div className="pr-empty" role="status">
      <p className="pr-empty__title">No pull requests here yet</p>
      <p className="pr-empty__text dim">{emptyGuidance(view)}</p>
      {projectError && <p className="pr-empty__error">⚠ Could not refresh this project: {projectError}</p>}
    </div>
  );
}

interface AllHiddenProps {
  /** Pull requests the text filter lets through, all hidden by switches. */
  count: number;
  filtered: boolean;
  /** Counted once each, drafts first, then age, then milestone, then area (as in the hidden line). */
  byDraft: number;
  byAge: number;
  byMilestone: number;
  byArea: number;
  /**
   * Whether each switch hides anything here, counting pull requests also
   * hidden by another switch, so following one action never lands on a list
   * still empty for a reason not named.
   */
  draftsHide: boolean;
  /** Whether the created recently switch hides anything here, counted the same way. */
  recentHides: boolean;
  milestonesHide: boolean;
  /** The projects whose area switches hide something here, counted the same way. */
  areaRepos: string[];
  onShowDrafts(): void;
  onShowOlder(): void;
  onShowAllMilestones(): void;
  onShowAllAreas(repos: string[]): void;
}

function AllHidden(props: AllHiddenProps) {
  const { count, filtered, byDraft, byAge, byMilestone, byArea, draftsHide, recentHides, milestonesHide, areaRepos } =
    props;
  const one = count === 1;
  const what = one
    ? filtered
      ? "The one matching pull request is"
      : "The one pull request here is"
    : filtered
      ? `All ${count} matching pull requests are`
      : `All ${count} pull requests here are`;
  const byAnyArea = areaRepos.length > 0;
  const causes = [draftsHide, recentHides, milestonesHide, byAnyArea].filter(Boolean).length;
  const several = causes > 1;
  const areaWhere = areaRepos.length === 1 ? `areas of ${areaRepos[0]}` : "areas";
  const switchedOff = [milestonesHide ? "milestones" : null, byAnyArea ? areaWhere : null]
    .filter((p): p is string => p != null)
    .join(" or ");
  // Milestones and areas share one phrase ("in milestones or areas you have
  // switched off"); drafts and age each have their own.
  const reasons = [
    draftsHide ? "drafts" : null,
    recentHides ? "older than 3 days" : null,
    switchedOff ? `in ${switchedOff} you have switched off` : null,
  ].filter((p): p is string => p != null);
  let title: string;
  if (reasons.length > 1) {
    // "drafts or in milestones you have switched off", "drafts, older than
    // 3 days or in areas you have switched off".
    title = one
      ? `${what} hidden by more than one filter`
      : `${what} ${reasons.slice(0, -1).join(", ")} or ${reasons[reasons.length - 1]}`;
  } else if (draftsHide) title = `${what} ${one ? "a draft" : "drafts"}, and drafts are hidden`;
  else if (recentHides) title = `${what} older than 3 days, and only recent pull requests are shown`;
  else title = `${what} in ${switchedOff} you have switched off`;
  return (
    <div className="pr-empty" role="status">
      <p className="pr-empty__title">{title}</p>
      {several && <p className="pr-empty__text dim">{hiddenNote({ byDraft, byAge, byMilestone, byArea })}</p>}
      <div className="pr-empty__actions">
        {draftsHide && (
          <button type="button" className="btn" onClick={props.onShowDrafts}>
            Show drafts
          </button>
        )}
        {recentHides && (
          <button type="button" className="btn" onClick={props.onShowOlder}>
            {several ? "Show older pull requests" : "Show all"}
          </button>
        )}
        {milestonesHide && (
          <button type="button" className="btn" onClick={props.onShowAllMilestones}>
            {several ? "Show all milestones" : "Show all"}
          </button>
        )}
        {byAnyArea && (
          <button type="button" className="btn" onClick={() => props.onShowAllAreas(areaRepos)}>
            {several ? "Show all areas" : "Show all"}
          </button>
        )}
      </div>
    </div>
  );
}

function NoMatch({ query, onClear }: { query: string; onClear(): void }) {
  return (
    <div className="pr-empty" role="status">
      <p className="pr-empty__title">No pull requests match “{query.trim()}”</p>
      <button type="button" className="btn" onClick={onClear}>
        Clear filter
      </button>
    </div>
  );
}
