// Confirms a squash merge. On open it asks GitHub, fresh, whether the PR can
// be merged; only a ready answer offers the merge button. Merging cannot be
// undone, so this dialog is the only way to it.

import { useCallback, useEffect, useRef, useState } from "react";
import type { MergeCheck, PullRequest } from "../../types";
import { errorMessage, useApp } from "../../state/store";
import { commentsText, mergeWarnings } from "../../lib/merge";
import { Dialog } from "../ui/Dialog";
import "./MergeDialog.css";

type CheckState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "done"; check: MergeCheck };

export function MergeDialog({ pr, onClose }: { pr: PullRequest; onClose(): void }) {
  const mergeCheck = useApp((s) => s.mergeCheck);
  const squashMerge = useApp((s) => s.squashMerge);
  const [state, setState] = useState<CheckState>({ kind: "loading" });
  const [merging, setMerging] = useState(false);
  const [mergeError, setMergeError] = useState<string | null>(null);
  // Set synchronously, so a double click cannot send a second merge.
  const inFlight = useRef(false);
  const live = useRef(true);
  const dismiss = useRef<HTMLButtonElement>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  const prId = pr.id;

  const runCheck = useCallback(async () => {
    setState({ kind: "loading" });
    try {
      const check = await mergeCheck(prId);
      if (live.current) setState({ kind: "done", check });
    } catch (e) {
      if (live.current) setState({ kind: "failed", message: errorMessage(e) });
    }
  }, [mergeCheck, prId]);

  useEffect(() => {
    live.current = true;
    void runCheck();
    return () => {
      live.current = false;
    };
  }, [runCheck]);

  // The dialog cannot be dismissed while the merge request is running.
  const close = () => {
    if (!inFlight.current) onClose();
  };

  const retry = () => {
    dismiss.current?.focus();
    void runCheck();
  };

  const merge = async (check: MergeCheck) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setMerging(true);
    setMergeError(null);
    try {
      await squashMerge(check.id, check.headSha);
      inFlight.current = false;
      onClose();
    } catch (e) {
      inFlight.current = false;
      if (!live.current) return;
      setMerging(false);
      setMergeError(errorMessage(e));
    }
  };

  // Disabled buttons drop focus; put it back on the merge button after a rejection.
  useEffect(() => {
    if (mergeError) confirm.current?.focus();
  }, [mergeError]);

  const check = state.kind === "done" ? state.check : null;
  const ready = check?.merge.ready === true;

  return (
    <Dialog
      title={`Squash and merge #${pr.number}?`}
      description={pr.title}
      onClose={close}
      initialFocus={dismiss}
      size="md"
    >
      <div className="dialog-body merge-dialog" aria-busy={state.kind === "loading" || merging || undefined}>
        {state.kind === "loading" && (
          <p className="merge-dialog__status dim" role="status">
            Checking with GitHub…
          </p>
        )}
        {state.kind === "failed" && (
          <p className="dialog-error" role="alert">
            Could not check whether this pull request can be merged: {state.message}
          </p>
        )}
        {check && <CheckFacts check={check} />}
        {check && !ready && (
          <p className="merge-dialog__blocked" role="alert">
            <span className="merge-dialog__blocked-label">Cannot merge yet:</span>{" "}
            {check.merge.reason ?? "GitHub would not allow this merge."}
          </p>
        )}
        {check && ready && <Warnings pr={pr} />}
        {mergeError && (
          <p className="dialog-error" role="alert">
            The merge did not go through: {mergeError}
          </p>
        )}
      </div>
      <div className="dialog-actions">
        <button ref={dismiss} type="button" className="btn" onClick={close} disabled={merging}>
          {ready || state.kind === "loading" ? "Cancel" : "Close"}
        </button>
        {state.kind === "failed" && (
          <button type="button" className="btn" onClick={retry}>
            Retry
          </button>
        )}
        {check && ready && (
          <button
            ref={confirm}
            type="button"
            className="btn primary"
            onClick={() => void merge(check)}
            disabled={merging}
            aria-busy={merging || undefined}
          >
            {merging ? "Merging…" : "Squash and merge"}
          </button>
        )}
      </div>
    </Dialog>
  );
}

function CheckFacts({ check }: { check: MergeCheck }) {
  return (
    <dl className="merge-dialog__facts">
      <div>
        <dt>Into</dt>
        <dd className="mono">{check.baseBranch}</dd>
      </div>
      <div>
        <dt>Commit title</dt>
        <dd className="selectable">{check.commitTitle}</dd>
      </div>
      <div>
        <dt>Comments</dt>
        <dd>{commentsText(check.unresolvedThreads)}</dd>
      </div>
    </dl>
  );
}

function Warnings({ pr }: { pr: PullRequest }) {
  const warnings = mergeWarnings(pr);
  if (warnings.length === 0) return null;
  return (
    <ul className="merge-dialog__warnings" aria-label="Warnings">
      {warnings.map((w) => (
        <li key={w}>
          <span className="merge-dialog__warn-mark" aria-hidden="true">
            !
          </span>
          {w}
        </li>
      ))}
    </ul>
  );
}
