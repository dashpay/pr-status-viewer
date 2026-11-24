// The window frame: sidebar on the left; header and main area on the right.
// Decides what the main area shows (loading, errors, sign-in, no projects,
// or `children`, the PR table, with the run progress panel docked under it)
// and owns the project dialogs and the app-level keyboard shortcuts.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useApp } from "../../state/store";
import type { Unsubscribe } from "../../api";
import type { View } from "../../lib/select";
import { AddProjectDialog } from "../projects/AddProjectDialog";
import { FollowPicker } from "../projects/FollowPicker";
import { RemoveProjectDialog } from "../projects/RemoveProjectDialog";
import { CloneDialog } from "../run/CloneDialog";
import { RunPanel } from "../run/RunPanel";
import {
  GithubErrorBanner,
  LoadErrorState,
  NoProjectsState,
  SignedOutState,
  TableSkeleton,
} from "../states/States";
import { Header } from "./Header";
import { Sidebar } from "./Sidebar";
import { focusIsBusy, hasMod } from "../../lib/keys";
import "./Shell.css";

type OpenDialog =
  | { kind: "add" }
  | { kind: "browse"; repo: string }
  | { kind: "remove"; repo: string }
  | { kind: "clone"; repo: string }
  | null;

const VIEW_KEYS: Record<string, View> = {
  "1": { kind: "mine" },
  "2": { kind: "following" },
  "3": { kind: "needsMe" },
};

export function Shell({ children }: { children: ReactNode }) {
  const snapshot = useApp((s) => s.snapshot);
  const config = useApp((s) => s.config);
  const loading = useApp((s) => s.loading);
  const loadError = useApp((s) => s.loadError);
  const refresh = useApp((s) => s.refresh);
  const init = useApp((s) => s.init);
  const setView = useApp((s) => s.setView);

  const [dialog, setDialog] = useState<OpenDialog>(null);
  const closeDialog = useCallback(() => setDialog(null), []);
  const openAdd = useCallback(() => setDialog({ kind: "add" }), []);
  // The header's run status: where focus goes when the run panel hides itself.
  const runStatusRef = useRef<HTMLButtonElement>(null);

  // A retry after a failed first load re-runs init (the config never
  // arrived); keep its subscriptions so they can be released.
  const retrySubs = useRef<Unsubscribe | null>(null);
  useEffect(() => () => retrySubs.current?.(), []);
  const retryLoad = useCallback(async () => {
    if (config) return refresh();
    retrySubs.current?.();
    retrySubs.current = await init();
  }, [config, refresh, init]);

  useShortcuts({
    refresh: () => void refresh(),
    addProject: openAdd,
    setView,
    dialogOpen: dialog !== null,
  });

  let main: ReactNode;
  let banner: ReactNode = null;
  if (!snapshot && loadError && !loading) {
    main = <LoadErrorState error={loadError} onRetry={retryLoad} />;
  } else if (!snapshot) {
    main = <TableSkeleton />;
  } else if (snapshot.signInRequired) {
    main = <SignedOutState error={snapshot.github.error} onRetry={refresh} />;
  } else if (config && config.projects.length === 0) {
    main = <NoProjectsState onAdd={openAdd} />;
  } else {
    main = children;
    if (!snapshot.github.ok) {
      banner = (
        <GithubErrorBanner
          error={snapshot.github.error}
          lastSuccessAt={snapshot.github.lastSuccessAt}
          onRetry={refresh}
        />
      );
    }
  }

  return (
    <div className="shell">
      <Sidebar
        onAddProject={openAdd}
        onBrowse={(repo) => setDialog({ kind: "browse", repo })}
        onRemove={(repo) => setDialog({ kind: "remove", repo })}
        onClone={(repo) => setDialog({ kind: "clone", repo })}
      />
      <Header runStatusRef={runStatusRef} />
      <main className="shell-main" aria-label="Pull requests">
        {banner}
        <div className="shell-scroll">{main}</div>
        <RunPanel returnFocus={runStatusRef} />
      </main>

      {dialog?.kind === "add" && <AddProjectDialog onClose={closeDialog} />}
      {dialog?.kind === "browse" && <FollowPicker repo={dialog.repo} onClose={closeDialog} />}
      {dialog?.kind === "remove" && <RemoveProjectDialog repo={dialog.repo} onClose={closeDialog} />}
      {dialog?.kind === "clone" && <CloneDialog repo={dialog.repo} onClose={closeDialog} />}
    </div>
  );
}

interface ShortcutActions {
  refresh(): void;
  addProject(): void;
  setView(view: View): void;
  dialogOpen: boolean;
}

/** ⌘R refresh, ⌘N add project, ⌘1–3 views (Ctrl on Linux). */
function useShortcuts(actions: ShortcutActions) {
  const ref = useRef(actions);
  ref.current = actions;

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!hasMod(e) || e.altKey || e.shiftKey) return;
      const key = e.key.toLowerCase();
      const isOurs = key === "r" || key === "n" || key in VIEW_KEYS;
      if (!isOurs) return;
      // Never let ⌘R reload the web view, even when we ignore it.
      if (key === "r") e.preventDefault();
      if (e.defaultPrevented && key !== "r") return;
      if (ref.current.dialogOpen || focusIsBusy(e.target) || focusIsBusy()) return;
      e.preventDefault();
      if (e.repeat) return;
      if (key === "r") ref.current.refresh();
      else if (key === "n") ref.current.addProject();
      else ref.current.setView(VIEW_KEYS[key]);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
