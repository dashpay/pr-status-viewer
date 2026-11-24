// "Local clone…" from a project's menu: where the project's clone is, how it
// was found, which example apps it has, and a way to choose another folder.
import { useEffect, useRef, useState } from "react";
import { errorMessage, useApp } from "../../state/store";
import { PLATFORMS, PLATFORM_LABEL } from "../../lib/run";
import { Dialog } from "../ui/Dialog";
import "./CloneDialog.css";

export function CloneDialog({ repo, onClose }: { repo: string; onClose(): void }) {
  const project = useApp((s) => s.runTargets.find((t) => t.repo === repo));
  const loadRunTargets = useApp((s) => s.loadRunTargets);
  const pickFolder = useApp((s) => s.pickFolder);
  const setClone = useApp((s) => s.setClone);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chooseRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    void loadRunTargets();
  }, [loadRunTargets]);

  // Buttons stay focusable while busy (aria-disabled), so focus is never lost.
  const act = async (fn: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const choose = () =>
    act(async () => {
      const path = await pickFolder();
      if (path) await setClone(repo, path);
    });

  const clonePath = project?.clonePath ?? null;
  const apps = PLATFORMS.filter((p) => project?.platforms.includes(p)).map((p) => PLATFORM_LABEL[p]);

  return (
    <Dialog title="Local clone" description={repo} onClose={onClose} size="md">
      <div className="dialog-body">
        <dl className="clone-facts">
          <div className="clone-fact">
            <dt>Folder</dt>
            <dd>
              {clonePath ? (
                <span className="mono selectable clone-path">{clonePath}</span>
              ) : (
                <span className="dim">No local clone found</span>
              )}
            </dd>
          </div>
          {clonePath && (
            <div className="clone-fact">
              <dt>Chosen</dt>
              <dd>{project?.cloneIsSet ? "Set by you" : "Found automatically"}</dd>
            </div>
          )}
          {clonePath && (
            <div className="clone-fact">
              <dt>Example apps</dt>
              <dd>{apps.length ? apps.join(" and ") : "None found"}</dd>
            </div>
          )}
        </dl>
        {!clonePath && (
          <p className="clone-note dim">PR Status looks for clones among T3 Code's projects. Choose a folder to set one yourself.</p>
        )}
        {error && (
          <p className="dialog-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="dialog-actions clone-actions">
        {project?.cloneIsSet && (
          <button
            type="button"
            className="btn ghost clone-auto"
            aria-disabled={busy || undefined}
            onClick={() =>
              void act(async () => {
                await setClone(repo, null);
                // This button goes away; keep focus in the dialog.
                chooseRef.current?.focus();
              })
            }
          >
            Find automatically
          </button>
        )}
        <button ref={chooseRef} type="button" className="btn" aria-disabled={busy || undefined} onClick={() => void choose()}>
          Choose folder…
        </button>
        <button type="button" className="btn primary" onClick={onClose}>
          Done
        </button>
      </div>
    </Dialog>
  );
}
