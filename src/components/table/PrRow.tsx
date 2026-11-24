// One pull request: a two-line row (title block | finding bubbles | traits |
// time | CI) and, when open, its detail underneath. A draft's title line
// starts with a "Draft" tag and its title is quieter, so the live pull
// requests stand out down the list. A PR stacked on the row above it (or on
// a row further up, after its siblings) starts with a ↳ arrow and its title
// block steps in, one step per level.

import { memo, type CSSProperties } from "react";
import type { CiState, PullRequest } from "../../types";
import { nextStepRowLabel, subLineParts, timeCell } from "../../lib/format";
import { t3RowTag } from "../../lib/merge";
import { isFromNestedControl } from "../../lib/keys";
import { traitsOf } from "../../lib/traits";
import { rowMarkerText } from "../../lib/run";
import { rowNextStepWithCue } from "../../lib/selfReview";
import { stackedLabel, stackedTitle, type StackPlace } from "../../lib/stack";
import { useApp } from "../../state/store";
import { FindingBubbles } from "./FindingBubbles";
import { PrDetail } from "./PrDetail";
import { TraitButtons } from "./Traits";
import "./PrRow.css";

/** A DOM-safe id fragment for a PR id like `owner/name#123`. */
export function domId(prId: string): string {
  return prId.replace(/[^A-Za-z0-9_-]/g, "-");
}

const CI_TEXT: Record<CiState, { glyph: string; label: string }> = {
  passing: { glyph: "✓", label: "CI passing" },
  failing: { glyph: "✕", label: "CI failing" },
  pending: { glyph: "●", label: "CI running" },
  none: { glyph: "—", label: "No CI" },
};

/** Two branches joining, the merge drawn in GitHub's way. Decorative: the mark around it names the conflict. */
function ConflictIcon() {
  return (
    <svg viewBox="0 0 14 14" width="12" height="12" aria-hidden="true" focusable="false">
      <g fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round">
        <circle cx="3.6" cy="2.6" r="1.5" />
        <circle cx="3.6" cy="11.4" r="1.5" />
        <circle cx="10.6" cy="7.6" r="1.5" />
        <path d="M3.6 4.1v5.8M3.6 4.1c0 2.6 2.2 3.5 5.5 3.5" />
      </g>
    </svg>
  );
}

/** "Merge conflicts with v5.1-dev", for the conflict mark's tooltip and accessible name. */
export function conflictText(pr: PullRequest): string {
  return `Merge conflicts with ${pr.baseBranch || "the base branch"}`;
}

export function CiMark({ ci }: { ci: CiState }) {
  const { glyph, label } = CI_TEXT[ci];
  return (
    <span className={`ci ${ci}`} title={label}>
      <span aria-hidden="true">{ci === "none" ? "— " : `${glyph} CI`}</span>
      <span className="sr-only">{label}</span>
    </span>
  );
}

interface PrRowProps {
  pr: PullRequest;
  now: number;
  showRepo: boolean;
  expanded: boolean;
  focused: boolean;
  /** The one row in the list that takes Tab focus. */
  tabStop: boolean;
  /** Where it sits under the PR it is stacked on; undefined at the left edge. */
  stack?: StackPlace;
  onToggle(id: string): void;
  onFocusRow(id: string): void;
  rowRef(id: string, el: HTMLElement | null): void;
}

function PrRowImpl({ pr, now, showRepo, expanded, focused, tabStop, stack, onToggle, onFocusRow, rowRef }: PrRowProps) {
  const time = timeCell(pr, now);
  const [number, ...rest] = subLineParts(pr, now, { showRepo: false });
  // The next step comes last; "Your move" says what the move is when the gate wants the self-review.
  const next = nextStepRowLabel(pr);
  if (next != null && rest.length > 0 && rest[rest.length - 1] === next) {
    rest[rest.length - 1] = rowNextStepWithCue(pr, next);
  }
  const id = domId(pr.id);
  const traits = traitsOf(pr);
  // A string or null, so only the row being run re-renders as the job moves.
  const running = useApp((s) => rowMarkerText(s.runJob, pr.id));
  const t3Tag = t3RowTag(pr);
  const classes = ["pr-row"];
  if (expanded) classes.push("expanded");
  if (focused) classes.push("focused");
  if (pr.isDraft) classes.push("draft");
  if (stack) classes.push("stacked");

  return (
    <>
      <div
        ref={(el) => rowRef(pr.id, el)}
        className={classes.join(" ")}
        role="button"
        tabIndex={tabStop ? 0 : -1}
        aria-expanded={expanded}
        aria-controls={expanded ? `pr-detail-${id}` : undefined}
        data-pr-row={pr.id}
        onClick={(e) => {
          // A trait icon (or the menu it opened) handles its own clicks.
          if (isFromNestedControl(e.target, e.currentTarget)) return;
          onToggle(pr.id);
        }}
        onFocus={() => onFocusRow(pr.id)}
      >
        <div
          className="pr-row__main"
          style={stack ? ({ "--stack-depth": stack.depth } as CSSProperties) : undefined}
          data-stack-depth={stack?.depth}
        >
          <div className="pr-row__title">
            {stack && (
              <>
                <span
                  className="pr-row__stacked"
                  role="img"
                  aria-label={stackedLabel(stack.parent)}
                  title={stackedTitle(stack.parent)}
                >
                  ↳
                </span>
                {/* So the row's name reads "Stacked on #6132, feat(sdk): ...". */}
                <span className="sr-only">, </span>
              </>
            )}
            {pr.isDraft && (
              <>
                <span className="pr-row__draft" title="Draft: not yet marked ready for review">
                  Draft
                </span>
                {/* So the row's name reads "Draft, feat(sdk): ..." and not "Draftfeat(sdk)". */}
                <span className="sr-only">, </span>
              </>
            )}
            {pr.needsMe && (
              <span
                className="pr-row__needs"
                role="img"
                aria-label={`Needs you: ${pr.needsMeReason ?? "your action"}`}
                title={`Needs you: ${pr.needsMeReason ?? "your action"}`}
              />
            )}
            <span className="pr-row__title-text">{pr.title}</span>
            {running && (
              <span className="pr-row__run" title="Building and opening this pull request's example app">
                {running}
              </span>
            )}
            {t3Tag && (
              <span className={`pr-row__t3 ${t3Tag.state}`} title={t3Tag.title}>
                {t3Tag.label}
              </span>
            )}
            {pr.hasConflicts && (
              <span className="pr-row__conflicts" role="img" aria-label={conflictText(pr)} title={conflictText(pr)}>
                <ConflictIcon />
              </span>
            )}
          </div>
          <div className="pr-row__sub">
            {showRepo && <>{pr.repo} · </>}
            <span className="mono">{number}</span>
            {rest.map((part, i) => (
              <span key={i}> · {part}</span>
            ))}
          </div>
        </div>
        <div className="pr-row__status">
          <FindingBubbles pr={pr} />
        </div>
        <div className="pr-row__traits">
          <TraitButtons traits={traits} tabbable={tabStop} />
        </div>
        <div className="pr-row__time" title={time.title ?? undefined}>
          <span className={`pr-row__when${time.dim ? " dim" : ""}`}>
            {time.strong ? <strong>{time.text}</strong> : time.text}
          </span>
          {time.live && (
            <span className={`pr-row__live ${time.live.tone}`}>
              {time.live.tone === "run" && <span className="dot" aria-hidden="true" />}
              {time.live.text}
            </span>
          )}
        </div>
        <div className="pr-row__ci">
          <CiMark ci={pr.ci} />
        </div>
      </div>
      {expanded && <PrDetail pr={pr} now={now} id={`pr-detail-${id}`} />}
    </>
  );
}

export const PrRow = memo(PrRowImpl);
