// The run progress panel, docked under the list. Not modal: the list stays
// usable above it. Shows the job's state, its steps and its log.
import { memo, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import type { RunJob, RunStep } from "../../types";
import { errorMessage, useApp } from "../../state/store";
import { useNow } from "../../lib/useNow";
import {
  RUN_STATE_TEXT,
  STEP_STATUS_TEXT,
  jobElapsedText,
  jobHeading,
  shortSha,
  stepDurationText,
} from "../../lib/run";
import "./RunPanel.css";

export const RUN_PANEL_ID = "run-panel";

interface RunPanelProps {
  /** Where focus goes when the panel hides itself: the header's run status. */
  returnFocus?: RefObject<HTMLElement | null>;
}

export function RunPanel({ returnFocus }: RunPanelProps) {
  const job = useApp((s) => s.runJob);
  const open = useApp((s) => s.runPanelOpen);
  if (!job || !open) return null;
  return <Panel job={job} returnFocus={returnFocus} />;
}

function Panel({ job, returnFocus }: { job: RunJob; returnFocus?: RefObject<HTMLElement | null> }) {
  const runCancel = useApp((s) => s.runCancel);
  const runOpenLog = useApp((s) => s.runOpenLog);
  const setRunPanelOpen = useApp((s) => s.setRunPanelOpen);
  const running = job.state === "running";
  const now = useNow(running);
  const [message, setMessage] = useState<string | null>(null);
  const elapsed = jobElapsedText(job, now);

  const act = async (fn: () => Promise<void>) => {
    setMessage(null);
    try {
      await fn();
    } catch (e) {
      setMessage(errorMessage(e));
    }
  };

  const hide = () => {
    setRunPanelOpen(false);
    returnFocus?.current?.focus();
  };

  // Keys pressed in the panel (arrows in the log, Esc) are the panel's, not
  // the list's. Shortcuts with a modifier still reach the app.
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
    }
  };

  return (
    <section id={RUN_PANEL_ID} className={`run-panel ${job.state}`} aria-labelledby="run-panel-title" onKeyDown={onKeyDown}>
      <div className="run-panel__head">
        <div className="run-panel__titles">
          <h2 id="run-panel-title" className="run-panel__title">
            {jobHeading(job)}
          </h2>
          <span className={`run-panel__state ${job.state}`}>
            <StateGlyph job={job} />
            {RUN_STATE_TEXT[job.state]}
            {elapsed && <span className="run-panel__elapsed"> · {elapsed}</span>}
          </span>
          <span className="run-panel__source dim">
            {job.sourceKind === "prHead" ? "Pull request head" : "Your worktree"}
            {job.sha && (
              <>
                {" · "}
                <span className="mono">{shortSha(job.sha)}</span>
              </>
            )}
          </span>
        </div>
        <div className="run-panel__actions">
          {message && (
            <span className="run-panel__msg" role="alert">
              {message}
            </span>
          )}
          {running && (
            <button type="button" className="btn" onClick={() => void act(runCancel)}>
              Cancel
            </button>
          )}
          {job.logPath && (
            <button type="button" className="btn" title={job.logPath} onClick={() => void act(runOpenLog)}>
              Open full log
            </button>
          )}
          <button type="button" className="btn ghost" onClick={hide}>
            Hide
          </button>
        </div>
      </div>
      {job.state === "failed" && job.error && (
        <p className="run-panel__error">
          <span aria-hidden="true">✕ </span>
          {job.error}
        </p>
      )}
      <div className="run-panel__body">
        <Steps job={job} now={now} />
        <LogView />
      </div>
    </section>
  );
}

function StateGlyph({ job }: { job: RunJob }) {
  if (job.state === "running") return <span className="spinner" aria-hidden="true" />;
  const glyph = job.state === "succeeded" ? "✓" : job.state === "failed" ? "✕" : "■";
  return (
    <span className="run-panel__glyph" aria-hidden="true">
      {glyph}
    </span>
  );
}

/**
 * A step's status in words. The step a cancelled job was in the middle of
 * reads "Stopped": the backend reports it as pending again, with the time it
 * started, because it did not fail.
 */
function stepStatus(job: RunJob, step: RunStep): { key: string; text: string; glyph: string } {
  const begun = step.status !== "done" && (step.status !== "pending" || step.startedAt != null);
  if (job.state === "cancelled" && begun) {
    return { key: "stopped", text: "Stopped", glyph: "■" };
  }
  const glyph = { done: "✓", running: "", pending: "○", failed: "✕" }[step.status];
  return { key: step.status, text: STEP_STATUS_TEXT[step.status], glyph };
}

function Steps({ job, now }: { job: RunJob; now: number }) {
  return (
    <ol className="run-steps" aria-label="Steps">
      {job.steps.map((step) => {
        const status = stepStatus(job, step);
        const duration = stepDurationText(step, now);
        return (
          <li
            key={step.key}
            className={`run-step ${status.key}`}
            aria-current={step.status === "running" && job.state === "running" ? "step" : undefined}
          >
            <span className="run-step__icon" aria-hidden="true">
              {status.key === "running" ? <span className="spinner" /> : status.glyph}
            </span>
            <span className="run-step__label">{step.label}</span>
            <span className="run-step__status">
              {status.text}
              {duration && <span className="run-step__time"> · {duration}</span>}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

/** How close to the bottom counts as "at the bottom", in pixels. */
const STICK_SLACK = 24;

/**
 * The log, in one `<pre>`: a batch of lines changes one text node, so 2000
 * lines stay smooth. Follows the bottom unless the user has scrolled up.
 */
function LogView() {
  const lines = useApp((s) => s.runLog);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const [away, setAway] = useState(false);

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const onScroll = () => {
    const el = box.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_SLACK;
    stick.current = atBottom;
    setAway(!atBottom);
  };

  const jump = () => {
    const el = box.current;
    if (!el) return;
    stick.current = true;
    el.scrollTop = el.scrollHeight;
    setAway(false);
  };

  return (
    <div className="run-log">
      <div
        ref={box}
        className="run-log__scroll"
        role="region"
        aria-label="Log"
        aria-live="off"
        tabIndex={0}
        onScroll={onScroll}
      >
        <LogText lines={lines} />
      </div>
      {away && (
        <button type="button" className="btn run-log__jump" onClick={jump}>
          Jump to latest
        </button>
      )}
    </div>
  );
}

const LogText = memo(function LogText({ lines }: { lines: string[] }) {
  const text = useMemo(() => lines.join("\n"), [lines]);
  if (lines.length === 0) return <p className="run-log__empty dim">No output yet</p>;
  return <pre className="run-log__text selectable">{text}</pre>;
});
