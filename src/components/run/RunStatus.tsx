// The run's compact status in the header ("Running on iOS · Build the SDK"),
// which shows and hides the progress panel, plus the one live region that
// announces the run's state changes (never its log lines).
import { forwardRef } from "react";
import type { RunJob } from "../../types";
import { useApp } from "../../state/store";
import { headerStatusText } from "../../lib/run";
import { RUN_PANEL_ID } from "./RunPanel";
import "./RunStatus.css";

export const RunStatus = forwardRef<HTMLButtonElement>(function RunStatus(_props, ref) {
  const job = useApp((s) => s.runJob);
  const open = useApp((s) => s.runPanelOpen);
  const setRunPanelOpen = useApp((s) => s.setRunPanelOpen);
  return (
    <>
      {job && (
        <button
          ref={ref}
          type="button"
          className={`run-status ${job.state}`}
          aria-expanded={open}
          aria-controls={open ? RUN_PANEL_ID : undefined}
          title={open ? "Hide run progress" : "Show run progress"}
          onClick={() => setRunPanelOpen(!open)}
        >
          <Glyph job={job} />
          <span className="run-status__text">{headerStatusText(job)}</span>
        </button>
      )}
      {/* Always mounted, so the first change is announced too. */}
      <span className="sr-only run-status__live" aria-live="polite" aria-atomic="true">
        {job ? announcement(job) : ""}
      </span>
    </>
  );
});

/** What a screen reader hears when the run changes: its state and step, and on failure, why. */
function announcement(job: RunJob): string {
  const text = headerStatusText(job);
  return job.state === "failed" && job.error ? `${text}: ${job.error}` : text;
}

function Glyph({ job }: { job: RunJob }) {
  if (job.state === "running") return <span className="spinner" aria-hidden="true" />;
  const glyph = job.state === "succeeded" ? "✓" : job.state === "failed" ? "✕" : "■";
  return (
    <span className="run-status__glyph" aria-hidden="true">
      {glyph}
    </span>
  );
}
