// The row's status cell: a red bubble for blockers and a yellow one for
// suggestions, in fixed slots, or a short note when there is nothing to count.
// A bubble reads "👎1 ⏳3 👍2 / 6": closed without the reviewer's agreement,
// awaiting re-review, settled with agreement, then the total. Its accessible
// name and tooltip say the same in words.

import type { PullRequest } from "../../types";
import { DRAFT_SKIPPED_NOTE, countText, statusCell, type Bubble } from "../../lib/findings";

/** A small thumb drawn in currentColor, so it matches the text on every platform and theme. */
export function Thumb({ down = false }: { down?: boolean }) {
  return (
    <svg className="tally__icon" viewBox="0 0 12 12" aria-hidden="true" focusable="false">
      <g transform={down ? "rotate(180 6 6)" : undefined} fill="currentColor">
        <rect x="0.6" y="5.2" width="2.6" height="6.2" rx="0.7" />
        <path d="M4.1 5.4 6.3 1.3c.35-.65 1.4-.45 1.42.32L7.6 4.4h2.75c.8 0 1.36.74 1.16 1.5l-1.08 4.6c-.15.55-.6.9-1.15.9H4.1Z" />
      </g>
    </svg>
  );
}

/**
 * A rabbit's head, drawn in currentColor at the thumbs' size: two tall ears
 * over a round head with two eyes cut out. Marks a bubble whose total
 * includes CodeRabbit's findings.
 */
export function Rabbit() {
  return (
    <svg className="tally__icon tally__rabbit" viewBox="0 0 12 12" aria-hidden="true" focusable="false">
      <g fill="currentColor">
        <ellipse cx="4.3" cy="3.4" rx="1.2" ry="3" transform="rotate(-13 4.3 3.4)" />
        <ellipse cx="7.7" cy="3.4" rx="1.2" ry="3" transform="rotate(13 7.7 3.4)" />
        <path
          fillRule="evenodd"
          d="M2.3 8.4a3.7 3.3 0 1 0 7.4 0a3.7 3.3 0 1 0-7.4 0ZM4.1 8.1a.65.65 0 1 0 1.3 0a.65.65 0 1 0-1.3 0ZM6.6 8.1a.65.65 0 1 0 1.3 0a.65.65 0 1 0-1.3 0Z"
        />
      </g>
    </svg>
  );
}

/**
 * An hourglass, drawn in currentColor at the thumbs' size: a frame top and
 * bottom, the last of the sand in the upper bulb and a full lower bulb.
 * Marks findings whose threads were resolved after a push the reviewer has
 * not re-reviewed yet.
 */
export function Hourglass() {
  return (
    <svg className="tally__icon tally__hourglass" viewBox="0 0 12 12" aria-hidden="true" focusable="false">
      <g fill="currentColor">
        <rect x="1.8" y="0.4" width="8.4" height="1.5" rx="0.6" />
        <rect x="1.8" y="10.1" width="8.4" height="1.5" rx="0.6" />
        <path
          fillRule="evenodd"
          d="M2.9 1.9h6.2v1.3c0 1.45-1.5 2.2-2.25 2.8c.75.6 2.25 1.35 2.25 2.8v1.3H2.9V8.8c0-1.45 1.5-2.2 2.25-2.8C4.4 5.4 2.9 4.65 2.9 3.2ZM4.1 3h3.8c-.1.75-.95 1.3-1.9 1.85C5.05 4.3 4.2 3.75 4.1 3Z"
        />
      </g>
    </svg>
  );
}

export function FindingBubble({ bubble }: { bubble: Bubble }) {
  const { notAgreed, awaiting, agreed } = bubble;
  const classes = ["tally", bubble.severity, bubble.settled ? "quiet" : "open"];
  if (bubble.fit !== "normal") classes.push(bubble.fit);
  return (
    <span className={classes.join(" ")} role="img" aria-label={bubble.label} title={bubble.label}>
      {bubble.coderabbit && <Rabbit />}
      {notAgreed != null && (
        <span className="tally__part down">
          <Thumb down />
          {countText(notAgreed)}
        </span>
      )}
      {awaiting != null && (
        <span className="tally__part wait">
          <Hourglass />
          {countText(awaiting)}
        </span>
      )}
      {agreed != null && (
        <span className="tally__part up">
          <Thumb />
          {countText(agreed)}
        </span>
      )}
      {notAgreed == null && awaiting == null && agreed == null && <span className="tally__part">0</span>}
      <span className="tally__total">
        <span className="tally__slash">/</span>
        {countText(bubble.total)}
      </span>
    </span>
  );
}

/** Blockers left, suggestions right, each in its own slot whether or not it has a bubble. */
export function FindingBubbles({ pr }: { pr: PullRequest }) {
  const cell = statusCell(pr);
  if (cell.kind === "note") {
    return cell.tone === "draft" ? (
      <span className="pill draft pr-row__note" title={DRAFT_SKIPPED_NOTE}>
        {cell.text}
      </span>
    ) : (
      <span className="dim pr-row__note" title={cell.text}>
        {cell.text}
      </span>
    );
  }
  return (
    <>
      <span className="pr-row__slot" data-slot="blocking">
        {cell.blocking && <FindingBubble bubble={cell.blocking} />}
      </span>
      <span className="pr-row__slot" data-slot="suggestion">
        {cell.suggestion && <FindingBubble bubble={cell.suggestion} />}
      </span>
    </>
  );
}
