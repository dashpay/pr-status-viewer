// Wording and decisions for posting `/self-reviewed <sha>` from the app: the
// button in the open PR's detail, its confirmation, the palette entry and the
// row's cue. Pure, like format.ts: anything that depends on the clock takes `now`.

import type { PullRequest, SelfReviewState } from "../types";
import { relativeAgo } from "./format";

/** The slash command the PR Hygiene gate reads. Shown in the monospace font wherever it appears. */
export const SELF_REVIEW_COMMAND = "/self-reviewed";

/** The label of the detail's button, or null when there is nothing to offer. */
export function selfReviewButtonLabel(state: SelfReviewState): string | null {
  switch (state) {
    case "unavailable":
      return null;
    case "wanted":
    case "possible":
      return `Post ${SELF_REVIEW_COMMAND}`;
    case "posted":
      return "Post again";
  }
}

/** "Self-review posted 3 min ago, waiting for the gate". */
export function postedText(postedAt: string | null, now: number): string {
  const ago = relativeAgo(postedAt, now);
  return ago ? `Self-review posted ${ago}, waiting for the gate` : "Self-review posted, waiting for the gate";
}

/** True when the gate's sentence asks for the self-review. */
export function gateAsksForSelfReview(gate: string | null): boolean {
  return gate != null && gate.toLowerCase().includes("/self-review");
}

/** The palette command for a PR, or null when it has nothing to offer. */
export function selfReviewPaletteLabel(pr: PullRequest): string | null {
  switch (pr.selfReview.state) {
    case "unavailable":
      return null;
    case "posted":
      return `Post ${SELF_REVIEW_COMMAND} again on #${pr.number}`;
    default:
      return `Post ${SELF_REVIEW_COMMAND} on #${pr.number}`;
  }
}

/**
 * The row's next-step label with the self-review cue: "Your move" becomes
 * "Your move: self-review" when the gate wants the self-review. Anything else
 * comes back unchanged.
 */
export function rowNextStepWithCue(pr: PullRequest, label: string): string {
  return pr.selfReview.state === "wanted" ? `${label}: self-review` : label;
}

/** A piece of text, marked when it is the slash command, so it can be set in the monospace font. */
export interface TextPiece {
  text: string;
  command: boolean;
}

/** Splits text around each occurrence of the slash command. */
export function splitCommand(text: string): TextPiece[] {
  const out: TextPiece[] = [];
  let rest = text;
  for (;;) {
    const i = rest.indexOf(SELF_REVIEW_COMMAND);
    if (i < 0) break;
    if (i > 0) out.push({ text: rest.slice(0, i), command: false });
    out.push({ text: SELF_REVIEW_COMMAND, command: true });
    rest = rest.slice(i + SELF_REVIEW_COMMAND.length);
  }
  if (rest) out.push({ text: rest, command: false });
  return out;
}
