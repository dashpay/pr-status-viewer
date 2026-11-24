// Left column: views, projects (each with its area switches), add project,
// and the review bot's status.
import { useMemo, useRef, useState, type RefObject } from "react";
import { useApp } from "../../state/store";
import { sameView, viewCounts, type View } from "../../lib/select";
import { NO_AREA, NO_AREA_LABEL, areaCounts, areaVisible } from "../../lib/areas";
import type { FeedHealth, ProjectSnapshot } from "../../types";
import { Menu, MenuItem } from "../ui/Menu";
import { AreaFilter } from "./AreaFilter";
import "./Sidebar.css";

/** Which projects have their area list open. Per viewer, a convenience only. */
const OPEN_AREAS_KEY = "pr-status.sidebar.areas-open.v1";

function loadOpenAreas(): string[] {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(OPEN_AREAS_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter((r): r is string => typeof r === "string") : [];
  } catch {
    return [];
  }
}

function saveOpenAreas(repos: string[]): void {
  try {
    localStorage.setItem(OPEN_AREAS_KEY, JSON.stringify(repos));
  } catch {
    // Storage can be unavailable; the list then stays open for this session only.
  }
}

export interface SidebarProps {
  onAddProject(): void;
  onBrowse(repo: string): void;
  onRemove(repo: string): void;
  /** Opens the "Local clone" dialog. Optional so older callers keep working. */
  onClone?(repo: string): void;
}

const VIEWS: { view: View; label: string; key: "mine" | "following" | "needsMe" }[] = [
  { view: { kind: "mine" }, label: "Mine", key: "mine" },
  { view: { kind: "following" }, label: "Following", key: "following" },
  { view: { kind: "needsMe" }, label: "Needs me", key: "needsMe" },
];

export function Sidebar({ onAddProject, onBrowse, onRemove, onClone }: SidebarProps) {
  const snapshot = useApp((s) => s.snapshot);
  const projects = useApp((s) => s.config?.projects);
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const counts = viewCounts(snapshot);
  const byRepo = new Map<string, ProjectSnapshot>(snapshot?.projects.map((p) => [p.repo, p]) ?? []);
  const repos = projects ?? snapshot?.projects.map((p) => p.repo) ?? [];
  const [openAreas, setOpenAreas] = useState<string[]>(loadOpenAreas);
  const toggleOpen = (repo: string) =>
    setOpenAreas((open) => {
      const next = open.includes(repo) ? open.filter((r) => r !== repo) : [...open, repo];
      saveOpenAreas(next);
      return next;
    });

  return (
    <aside className="sidebar" aria-label="Views and projects">
      <nav className="sidebar-section" aria-labelledby="sidebar-views">
        <h2 id="sidebar-views" className="sidebar-heading">
          Views
        </h2>
        <ul className="nav-list">
          {VIEWS.map(({ view: v, label, key }) => {
            const on = sameView(view, v);
            return (
              <li key={key} className="nav-row">
                <button
                  type="button"
                  className={`nav${on ? " on" : ""}`}
                  aria-current={on ? "page" : undefined}
                  onClick={() => setView(v)}
                >
                  <span className="nav-label">{label}</span>
                  <span className="n">
                    {snapshot ? counts[key] : ""}
                    <span className="sr-only"> pull requests</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </nav>

      <nav className="sidebar-section sidebar-projects" aria-labelledby="sidebar-projects">
        <h2 id="sidebar-projects" className="sidebar-heading">
          Projects
        </h2>
        <ul className="nav-list nav-scroll">
          {repos.map((repo) => (
            <ProjectRow
              key={repo}
              repo={repo}
              project={byRepo.get(repo)}
              count={counts.byProject[repo]}
              on={view.kind === "project" && view.repo === repo}
              areasOpen={openAreas.includes(repo)}
              onToggleAreas={() => toggleOpen(repo)}
              onSelect={() => setView({ kind: "project", repo })}
              onBrowse={() => onBrowse(repo)}
              onRemove={() => onRemove(repo)}
              onClone={onClone && (() => onClone(repo))}
            />
          ))}
        </ul>
        <button type="button" className="nav add" onClick={onAddProject}>
          + Add project
        </button>
      </nav>

      {snapshot && <FeedStatus feed={snapshot.feed} />}
    </aside>
  );
}

interface ProjectRowProps {
  repo: string;
  project: ProjectSnapshot | undefined;
  count: number | undefined;
  on: boolean;
  areasOpen: boolean;
  onToggleAreas(): void;
  onSelect(): void;
  onBrowse(): void;
  onRemove(): void;
  onClone?(): void;
}

const EMPTY: string[] = [];

function ProjectRow({
  repo,
  project,
  count,
  on,
  areasOpen,
  onToggleAreas,
  onSelect,
  onBrowse,
  onRemove,
  onClone,
}: ProjectRowProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const more = useRef<HTMLButtonElement>(null);
  const disclose = useRef<HTMLButtonElement>(null);
  // Set when the user opens the list, so only that scrolls it into view.
  const openedHere = useRef(false);
  const hidden = useApp((s) => s.hiddenAreas[repo] ?? EMPTY);
  const toggleArea = useApp((s) => s.toggleArea);
  const showAllAreas = useApp((s) => s.showAllAreas);
  const prs = project?.prs;
  const areas = useMemo(() => areaCounts(prs ?? []), [prs]);
  // Worth a list only when there is something to choose, or something to undo.
  const expandable = areas.length >= 2 || hidden.length > 0;
  const open = expandable && areasOpen;
  const hiddenPrs = useMemo(() => {
    const switches = { [repo]: hidden };
    return (prs ?? []).filter((pr) => !areaVisible(pr, switches)).length;
  }, [prs, repo, hidden]);
  const panelId = `areas-${repo.replace(/[^A-Za-z0-9_-]/g, "-")}`;

  const offNames = hidden.map((k) => (k === NO_AREA ? NO_AREA_LABEL : k));
  const filterTip =
    hidden.length > 0
      ? `Areas switched off: ${offNames.join(", ")}. ${hiddenPrs} of ${count ?? 0} listed pull requests hidden.`
      : null;
  const tooltip = project
    ? `${count ?? 0} listed · ${project.openCount} open in ${repo}${hidden.length ? ` · ${hiddenPrs} hidden by area` : ""}`
    : repo;

  const choose = (action: () => void) => {
    setMenuOpen(false);
    // Focus the ⋯ button first, so the dialog returns focus there on close.
    more.current?.focus();
    action();
  };

  return (
    <li className={`nav-row project${menuOpen ? " menu-open" : ""}`}>
      <div className="project-line">
        {expandable && (
          <button
            ref={disclose}
            type="button"
            className="nav-disclose"
            aria-expanded={open}
            aria-controls={open ? panelId : undefined}
            aria-label={`Areas in ${repo}`}
            title={open ? "Hide areas" : "Show areas"}
            onClick={() => {
              openedHere.current = !open;
              onToggleAreas();
            }}
          >
            <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
              <path
                d="M3.5 2 L7 5 L3.5 8"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        )}
        <button
          type="button"
          className={`nav${on ? " on" : ""}`}
          aria-current={on ? "page" : undefined}
          title={tooltip}
          onClick={onSelect}
        >
          <span className="nav-label">{repo}</span>
          {project?.error && (
            <span className="nav-warn" title={project.error}>
              ⚠<span className="sr-only">Could not load: {project.error}</span>
            </span>
          )}
          {filterTip && (
            <span className="nav-filter" title={filterTip}>
              <span aria-hidden="true">{hidden.length} off</span>
              <span className="sr-only">
                , {hidden.length} {hidden.length === 1 ? "area" : "areas"} switched off,
              </span>
            </span>
          )}
          <span className="n">
            {count ?? ""}
            <span className="sr-only"> pull requests</span>
          </span>
        </button>
        <button
          ref={more}
          type="button"
          className="nav-more"
          aria-label={`Actions for ${repo}`}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((o) => !o)}
        >
          ⋯
        </button>
        {menuOpen && (
          <ProjectMenu
            anchor={more}
            repo={repo}
            onClose={(refocus) => {
              setMenuOpen(false);
              if (refocus) more.current?.focus();
            }}
            onBrowse={() => choose(onBrowse)}
            onRemove={() => choose(onRemove)}
            onClone={onClone && (() => choose(onClone))}
          />
        )}
      </div>
      {open && (
        <AreaFilter
          id={panelId}
          repo={repo}
          areas={areas}
          hidden={hidden}
          scrollOnOpen={openedHere.current}
          onToggle={(key) => toggleArea(repo, key)}
          onAll={() => showAllAreas(repo)}
          onNone={() => {
            for (const a of areas) if (!hidden.includes(a.key)) toggleArea(repo, a.key);
          }}
          onClose={() => {
            onToggleAreas();
            disclose.current?.focus();
          }}
        />
      )}
    </li>
  );
}

interface ProjectMenuProps {
  anchor: RefObject<HTMLButtonElement | null>;
  repo: string;
  onClose(refocus: boolean): void;
  onBrowse(): void;
  onRemove(): void;
  onClone?(): void;
}

function ProjectMenu({ anchor, repo, onClose, onBrowse, onRemove, onClone }: ProjectMenuProps) {
  return (
    <Menu anchor={anchor} label={`Actions for ${repo}`} className="project-menu" onClose={onClose}>
      <MenuItem onSelect={onBrowse}>Browse open PRs</MenuItem>
      {onClone && <MenuItem onSelect={onClone}>Local clone…</MenuItem>}
      <MenuItem onSelect={onRemove} danger>
        Remove…
      </MenuItem>
    </Menu>
  );
}

function FeedStatus({ feed }: { feed: FeedHealth }) {
  const median = feed.medianReviewMinutes == null ? null : Math.round(feed.medianReviewMinutes);
  const hasNumbers = feed.ok || feed.dataAsOf != null;
  return (
    <section className="feed" aria-labelledby="feed-title">
      <h2 id="feed-title" className="feed-title">
        PastaClaw
      </h2>
      {hasNumbers && (
        <>
          <p>
            {feed.running} running · {feed.queued} queued
          </p>
          {median != null && <p>median review {median} min</p>}
        </>
      )}
      {feed.degraded && <p className="feed-degraded">Running on stand-in models</p>}
      {!feed.ok && (
        <p className="feed-error" title={feed.error ?? undefined}>
          <span aria-hidden="true">⚠ </span>
          {feed.error ?? "Review status unavailable"}
        </p>
      )}
    </section>
  );
}
