// Add a repository to the tracked projects, then open it.
import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { errorMessage, useApp } from "../../state/store";
import { Dialog } from "../ui/Dialog";

export function AddProjectDialog({ onClose }: { onClose(): void }) {
  const addProject = useApp((s) => s.addProject);
  const setView = useApp((s) => s.setView);
  const config = useApp((s) => s.config);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The project list before a successful add, to find the new entry. */
  const [addedFrom, setAddedFrom] = useState<string[] | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const errorId = useId();
  const fieldId = useId();

  // The store updates `config` before `addProject` resolves; select whatever
  // repo is new (the backend may have normalised the name), then close.
  useEffect(() => {
    if (!addedFrom) return;
    const repo = config?.projects.find((p) => !addedFrom.includes(p));
    if (repo) setView({ kind: "project", repo });
    onClose();
  }, [addedFrom, config, setView, onClose]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    if (!value.trim()) {
      setError("Enter a repository as owner/name or paste its GitHub link.");
      input.current?.focus();
      return;
    }
    const before = config?.projects ?? [];
    setBusy(true);
    setError(null);
    try {
      await addProject(value.trim());
      setAddedFrom(before);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
      input.current?.focus();
    }
  };

  return (
    <Dialog
      title="Add project"
      description="Track your open pull requests in a GitHub repository."
      onClose={onClose}
      initialFocus={input}
    >
      <form onSubmit={submit}>
        <div className="dialog-body">
          <label className="dialog-label" htmlFor={fieldId}>
            Repository
          </label>
          <input
            ref={input}
            id={fieldId}
            className="input mono"
            placeholder="owner/name or https://github.com/owner/name"
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={value}
            onChange={(e) => {
              setValue(e.target.value);
              if (error) setError(null);
            }}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
          />
          {error && (
            <p id={errorId} className="dialog-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={busy} aria-busy={busy || undefined}>
            {busy ? "Adding…" : "Add project"}
          </button>
        </div>
      </form>
    </Dialog>
  );
}
