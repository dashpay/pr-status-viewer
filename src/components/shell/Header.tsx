// The slim bar above the main area: view name, then whose pull requests are
// in view, run status, sync status, quick filter.
import type { Ref } from "react";
import { useApp } from "../../state/store";
import { contributorCount, viewCounts, type View } from "../../lib/select";
import { contributorTitle } from "../../lib/people";
import { shortcutLabel } from "../../lib/keys";
import { formatAgo, useNow } from "./time";
import { RunStatus } from "../run/RunStatus";
import { PeopleSwitcher } from "../people/PeopleSwitcher";
import "./Header.css";

export function viewTitle(view: View): string {
  switch (view.kind) {
    case "mine":
      return "Mine";
    case "following":
      return "Following";
    case "needsMe":
      return "Needs me";
    case "project":
      return view.repo;
    case "contributor":
      return contributorTitle(view.login);
  }
}

export function Header({ runStatusRef }: { runStatusRef?: Ref<HTMLButtonElement> }) {
  const view = useApp((s) => s.view);
  const project = useApp((s) =>
    s.view.kind === "project" ? s.snapshot?.projects.find((p) => p.repo === (s.view as { repo: string }).repo) : undefined,
  );

  const contributorOpen = useApp((s) =>
    s.view.kind === "contributor" ? contributorCount(viewCounts(s.snapshot), s.view.login) : null,
  );

  return (
    <header className="header">
      <h1 className="header-title" title={viewTitle(view)}>
        {viewTitle(view)}
      </h1>
      {project && <span className="header-sub dim">{project.openCount} open</span>}
      {contributorOpen != null && <span className="header-sub dim">{contributorOpen} open</span>}
      <span className="header-sp" />
      <PeopleSwitcher />
      <RunStatus ref={runStatusRef} />
      <SyncButton />
      <QuickFilterButton />
    </header>
  );
}

function SyncButton() {
  const syncing = useApp((s) => s.syncing);
  // The snapshot is rebuilt after every feed refresh too, so its own
  // timestamp would say "synced" while GitHub is failing.
  const syncedAt = useApp((s) => s.snapshot?.github.lastSuccessAt ?? null);
  const refresh = useApp((s) => s.refresh);
  const now = useNow(1000);
  const label = syncing ? "Syncing…" : syncedAt ? `Synced ${formatAgo(syncedAt, now)}` : "Not synced yet";

  return (
    <button
      type="button"
      className="header-sync"
      onClick={() => {
        if (!syncing) void refresh();
      }}
      aria-disabled={syncing || undefined}
      aria-busy={syncing || undefined}
      title={`Sync now (${shortcutLabel("R")})`}
    >
      {syncing && <span className="spinner" aria-hidden="true" />}
      <span>{label}</span>
    </button>
  );
}

function QuickFilterButton() {
  const setPaletteOpen = useApp((s) => s.setPaletteOpen);
  const keys = shortcutLabel("K");
  return (
    <button
      type="button"
      className="header-filter"
      onClick={() => setPaletteOpen(true)}
      aria-label={`Quick filter (${keys})`}
    >
      <span className="header-filter-text">Quick filter</span>
      <span className="kbd" aria-hidden="true">
        {keys}
      </span>
    </button>
  );
}
