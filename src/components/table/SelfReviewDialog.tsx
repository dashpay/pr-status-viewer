// Confirms posting `/self-reviewed <sha>`. It is the user's own statement on
// GitHub that they have read the final diff, so it is never one click: on open
// the dialog asks GitHub, fresh, what would be posted, and shows exactly that.

import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { PullRequest, SelfReviewCheck } from "../../types";
import { errorMessage, useApp } from "../../state/store";
import { exactLocal, relativeAgo } from "../../lib/format";
import { shortSha } from "../../lib/run";
import { SELF_REVIEW_COMMAND, gateAsksForSelfReview, splitCommand } from "../../lib/selfReview";
import { useNow } from "../../lib/useNow";
import { Dialog } from "../ui/Dialog";
import "./SelfReviewDialog.css";

type CheckState =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "done"; check: SelfReviewCheck };

/** Text with the slash command set in the monospace font. */
export function CommandText({ text }: { text: string }) {
  return (
    <>
      {splitCommand(text).map((p, i) =>
        p.command ? (
          <span key={i} className="mono">
            {p.text}
          </span>
        ) : (
          p.text
        ),
      )}
    </>
  );
}

export function SelfReviewDialog({ pr, onClose }: { pr: PullRequest; onClose(): void }) {
  const selfReviewCheck = useApp((s) => s.selfReviewCheck);
  const postSelfReview = useApp((s) => s.postSelfReview);
  const openUrl = useApp((s) => s.openUrl);
  const now = useNow();
  const [state, setState] = useState<CheckState>({ kind: "loading" });
  const [posting, setPosting] = useState(false);
  const [postError, setPostError] = useState<string | null>(null);
  // The commit the user was shown when a post was refused, to say the head moved.
  const [refusedSha, setRefusedSha] = useState<string | null>(null);
  // Set synchronously, so a double click cannot post twice.
  const inFlight = useRef(false);
  const live = useRef(true);
  const dismiss = useRef<HTMLButtonElement>(null);
  const prId = pr.id;

  const runCheck = useCallback(async () => {
    setState({ kind: "loading" });
    try {
      const check = await selfReviewCheck(prId);
      if (live.current) setState({ kind: "done", check });
    } catch (e) {
      if (live.current) setState({ kind: "failed", message: errorMessage(e) });
    }
  }, [selfReviewCheck, prId]);

  useEffect(() => {
    live.current = true;
    void runCheck();
    return () => {
      live.current = false;
    };
  }, [runCheck]);

  // The dialog cannot be dismissed while the comment is being posted.
  const close = () => {
    if (!inFlight.current) onClose();
  };

  const retry = () => {
    dismiss.current?.focus();
    void runCheck();
  };

  const post = async (check: SelfReviewCheck) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setPosting(true);
    setPostError(null);
    setRefusedSha(null);
    try {
      await postSelfReview(check.id, check.headSha);
      inFlight.current = false;
      onClose();
    } catch (e) {
      inFlight.current = false;
      if (!live.current) return;
      setPosting(false);
      setPostError(errorMessage(e));
      setRefusedSha(check.headSha);
      // New commits may have been pushed: ask again so the dialog shows the current head.
      void runCheck();
    }
  };

  // After a refusal focus goes to Cancel (once it is enabled again), not back to
  // the post button: the commit may have changed, and Enter must not post a
  // statement about a commit the user has not seen.
  useEffect(() => {
    if (postError && !posting) dismiss.current?.focus();
  }, [postError, posting]);

  const check = state.kind === "done" ? state.check : null;
  const canPost = check?.canPost === true;
  const headMoved = check != null && refusedSha != null && check.headSha !== refusedSha;

  return (
    <Dialog
      title={
        <>
          Post <span className="mono">{SELF_REVIEW_COMMAND}</span> on #{pr.number}?
        </>
      }
      description={pr.title}
      onClose={close}
      initialFocus={dismiss}
      size="md"
    >
      <div className="dialog-body self-review" aria-busy={state.kind === "loading" || posting || undefined}>
        {state.kind === "loading" && (
          <p className="self-review__status dim" role="status">
            Checking with GitHub…
          </p>
        )}
        {state.kind === "failed" && (
          <p className="dialog-error" role="alert">
            Could not check whether the self-review can be posted: {state.message}
          </p>
        )}
        {postError && (
          <p className="dialog-error" role="alert">
            The comment was not posted: {postError}
          </p>
        )}
        {check && <CheckBody check={check} now={now} headMoved={headMoved} />}
      </div>
      <div className="dialog-actions self-review__actions">
        {check && (
          <button
            type="button"
            className="btn ghost self-review__diff"
            title={check.diffUrl}
            onClick={() => void openUrl(check.diffUrl)}
          >
            Open the diff on GitHub
          </button>
        )}
        <button ref={dismiss} type="button" className="btn" onClick={close} disabled={posting}>
          {canPost || state.kind === "loading" ? "Cancel" : "Close"}
        </button>
        {state.kind === "failed" && (
          <button type="button" className="btn" onClick={retry}>
            Retry
          </button>
        )}
        {check && canPost && (
          <button
            type="button"
            className="btn primary"
            onClick={() => void post(check)}
            disabled={posting}
            aria-busy={posting || undefined}
          >
            {posting ? (
              "Posting…"
            ) : check.alreadyPostedAt ? (
              "Post again"
            ) : (
              <>
                Post <span className="mono">{SELF_REVIEW_COMMAND}</span>
              </>
            )}
          </button>
        )}
      </div>
    </Dialog>
  );
}

function CheckBody({ check, now, headMoved }: { check: SelfReviewCheck; now: number; headMoved: boolean }) {
  const short = shortSha(check.headSha);
  const postedAgo = relativeAgo(check.alreadyPostedAt, now);
  const labelId = useId();
  return (
    <>
      <dl className="self-review__facts">
        <div>
          <dt>Commit</dt>
          <dd className="mono" title={check.headSha}>
            {short}
            {headMoved && <span className="self-review__moved"> (new commits were pushed)</span>}
          </dd>
        </div>
        {check.gate && (
          <div>
            <dt>PR Hygiene says</dt>
            <dd className="self-review__gate selectable">
              <CommandText text={check.gate} />
            </dd>
          </div>
        )}
      </dl>
      {check.gate && !gateAsksForSelfReview(check.gate) && (
        <p className="self-review__note">
          The gate is not asking for the self-review now, so posting it will not move the gate on by itself.
        </p>
      )}
      {check.alreadyPostedAt && (
        <p className="self-review__note" title={exactLocal(check.alreadyPostedAt) ?? undefined}>
          You posted a self-review for this commit{postedAgo ? ` ${postedAgo}` : ""}.
        </p>
      )}
      {check.canPost ? (
        <>
          <p className="self-review__intro" id={labelId}>
            This comment will be posted on the pull request as you:
          </p>
          <pre className="self-review__comment mono selectable" aria-labelledby={labelId}>
            {check.comment}
          </pre>
          <p className="self-review__meaning">
            It is your statement that you have read the final diff of commit <span className="mono">{short}</span>{" "}
            yourself.
          </p>
        </>
      ) : (
        <p className="self-review__blocked" role="alert">
          <span className="self-review__blocked-label">Cannot post:</span>{" "}
          {check.reason ?? "GitHub would not accept the self-review now."}
        </p>
      )}
    </>
  );
}
