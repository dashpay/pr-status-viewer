// Confirms removing a tracked project.
import { useRef, useState } from "react";
import { errorMessage, useApp } from "../../state/store";
import { Dialog } from "../ui/Dialog";

export function RemoveProjectDialog({ repo, onClose }: { repo: string; onClose(): void }) {
  const removeProject = useApp((s) => s.removeProject);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cancel = useRef<HTMLButtonElement>(null);

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      await removeProject(repo);
      onClose();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={`Remove ${repo}?`}
      description="Its pull requests leave the list, and the ones you follow there are unfollowed. You can add it again later."
      onClose={onClose}
      initialFocus={cancel}
      size="sm"
    >
      {error && (
        <div className="dialog-body">
          <p className="dialog-error" role="alert">
            {error}
          </p>
        </div>
      )}
      <div className="dialog-actions">
        <button ref={cancel} type="button" className="btn" onClick={onClose}>
          Cancel
        </button>
        <button type="button" className="btn danger" onClick={remove} disabled={busy} aria-busy={busy || undefined}>
          {busy ? "Removing…" : "Remove project"}
        </button>
      </div>
    </Dialog>
  );
}
