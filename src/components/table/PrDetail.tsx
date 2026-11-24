// The open row's detail: review progress or queue facts, reviewers, actions.

import { useEffect, useId, useState, type ReactNode } from "react";
import type { BotQueued, BotReviewing, NextStep, PrTrait, PullRequest, ReviewStep, T3Thread } from "../../types";
import { useApp, errorMessage } from "../../state/store";
import {
  QUEUED_ESTIMATE_NOTE,
  approxSpan,
  botReviewerText,
  clockTime,
  exactLocal,
  exactSpan,
  findingsText,
  humanStateText,
  parseTime,
  relativeAgo,
} from "../../lib/format";
import { LAST_ROUND_NOTE, findingCountsText, hasFindings, hasReviewed } from "../../lib/findings";
import { commentsText, showMergeButton, sortT3Threads, t3StateText } from "../../lib/merge";
import { TRAIT_NAME, traitLinks, traitsOf } from "../../lib/traits";
import { postedText, selfReviewButtonLabel, SELF_REVIEW_COMMAND } from "../../lib/selfReview";
import { MergeDialog } from "./MergeDialog";
import { SelfReviewDialog } from "./SelfReviewDialog";
import { RunButtons } from "../run/RunButtons";
import { TraitIcon } from "./Traits";
import "./PrDetail.css";

interface PrDetailProps {
  pr: PullRequest;
  now: number;
  id: string;
}

export function PrDetail({ pr, now, id }: PrDetailProps) {
  const t3Available = useApp((s) => s.snapshot?.t3Available ?? false);
  const traits = traitsOf(pr);
  return (
    <div className="pr-detail" id={id} role="region" aria-label={`Details for #${pr.number}`}>
      {pr.bot.state === "reviewing" && <ReviewingDetail bot={pr.bot} now={now} />}
      {pr.bot.state === "queued" && <QueuedDetail bot={pr.bot} now={now} />}
      <dl className="pr-facts">
        {pr.needsMe && <Fact label="Needs you">{pr.needsMeReason ?? "Your action"}</Fact>}
        {pr.nextStep && <NextStepFact step={pr.nextStep} />}
        {pr.isDraft && pr.nextStep?.stage !== "draft" && <Fact label="Draft">Not yet marked ready for review</Fact>}
        <Fact label="Reviewers">
          <Reviewers pr={pr} />
        </Fact>
        {(hasReviewed(pr) || hasFindings(pr)) && (
          <Fact label="Findings">
            {findingCountsText(pr.findingCounts)} <span className="pr-fact__note">{LAST_ROUND_NOTE}</span>
          </Fact>
        )}
        {pr.lastReviewAt && <LastReviewFact at={pr.lastReviewAt} now={now} />}
        <Fact label="Comments">{commentsText(pr.unresolvedThreads)}</Fact>
        {traits.length > 0 && <TraitsFact traits={traits} />}
        {pr.milestone && <Fact label="Milestone">{pr.milestone}</Fact>}
        {(pr.t3Threads.length > 0 || t3Available) && <T3Fact threads={pr.t3Threads} now={now} />}
      </dl>
      <Actions pr={pr} now={now} />
    </div>
  );
}

interface FactProps {
  label: string;
  children: ReactNode;
  title?: string;
  className?: string;
}

function Fact({ label, children, title, className }: FactProps) {
  return (
    <div className={className ? `pr-fact ${className}` : "pr-fact"} title={title}>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** When the bot's last review came in: the exact local time, then how long ago. */
function LastReviewFact({ at, now }: { at: string; now: number }) {
  const exact = exactLocal(at);
  if (exact == null) return null;
  return <Fact label="Last review">{`${exact} · ${relativeAgo(at, now)}`}</Fact>;
}

/** The hygiene gate's next step: its label, then its full sentence. */
function NextStepFact({ step }: { step: NextStep }) {
  return (
    <Fact label="Next step" className="pr-fact--wide pr-next">
      <span className="pr-next__label">{step.label}</span>
      {step.detail && <span className="pr-next__detail selectable">{step.detail}</span>}
    </Fact>
  );
}

/** Each trait with its summary and every link, so a trait with several links is fully reachable here too. */
function TraitsFact({ traits }: { traits: PrTrait[] }) {
  const openUrl = useApp((s) => s.openUrl);
  return (
    <Fact label="Traits" className="pr-fact--wide pr-traits">
      <ul className="pr-traits__list">
        {traits.map((t) => {
          const links = traitLinks(t);
          return (
            <li key={t.kind} className="pr-traits__item">
              <span className="pr-traits__name">
                <TraitIcon kind={t.kind} />
                {TRAIT_NAME[t.kind]}
              </span>
              {t.summary && <span className="pr-traits__summary selectable">{t.summary}</span>}
              {links.length > 0 ? (
                <span className="pr-traits__links">
                  {links.map((l, i) => (
                    <button
                      key={`${i}:${l.url}`}
                      type="button"
                      className="pr-traits__link"
                      title={l.url}
                      onClick={() => void openUrl(l.url)}
                    >
                      {l.label}
                    </button>
                  ))}
                </span>
              ) : (
                <span className="pr-traits__none">No viewer link</span>
              )}
            </li>
          );
        })}
      </ul>
    </Fact>
  );
}

/** The PR's T3 Code threads: working ones first, then waiting, then idle. */
function T3Fact({ threads, now }: { threads: T3Thread[]; now: number }) {
  if (threads.length === 0) return <Fact label="T3 Code">No thread</Fact>;
  return (
    <Fact label="T3 Code" className="pr-fact--wide pr-t3">
      <ul className="pr-t3__list">
        {sortT3Threads(threads).map((t) => (
          <li key={t.threadId} className={`pr-t3__thread ${t.state}`}>
            <span className="pr-t3__title">{t.title}</span>
            <span className="pr-t3__state">{t3StateText(t, now)}</span>
          </li>
        ))}
      </ul>
    </Fact>
  );
}

function ReviewingDetail({ bot, now }: { bot: BotReviewing; now: number }) {
  const started = parseTime(bot.startedAt);
  const pct = bot.progress == null ? null : Math.round(Math.min(1, Math.max(0, bot.progress)) * 100);
  return (
    <>
      <div
        className={`meter pr-detail__meter${pct == null ? " unknown" : ""}`}
        role="progressbar"
        aria-label="Review progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct ?? undefined}
        aria-valuetext={pct == null ? "Progress not reported" : `${pct}%`}
      >
        <div className="fill" style={pct == null ? undefined : { width: `${pct}%` }} />
      </div>
      {bot.steps.length > 0 && <Steps steps={bot.steps} />}
      <dl className="pr-facts">
        {bot.tier && <Fact label="Tier">{bot.tier}</Fact>}
        {bot.waitedSeconds != null && <Fact label="Waited">{exactSpan(bot.waitedSeconds)}</Fact>}
        {started != null && (
          <Fact label="Running" title={`Started ${exactLocal(bot.startedAt)}`}>
            {exactSpan((now - started) / 1000)}
          </Fact>
        )}
        {bot.priority && <Fact label="Priority">yes</Fact>}
        {bot.findings && <Fact label="Findings so far">{findingsText(bot.findings)}</Fact>}
      </dl>
    </>
  );
}

const STEP_GLYPH: Record<ReviewStep["status"], string> = {
  done: "✓ ",
  running: "● ",
  failed: "✕ ",
  pending: "",
};

const STEP_STATUS_WORD: Record<ReviewStep["status"], string> = {
  done: "done",
  running: "running",
  failed: "failed",
  pending: "not started",
};

function Steps({ steps }: { steps: ReviewStep[] }) {
  return (
    <ol className="pr-steps" aria-label="Review steps">
      {steps.map((s) => (
        <li key={s.key} className={`pr-step ${s.status}`} aria-current={s.status === "running" ? "step" : undefined}>
          <span className="pr-step__label">
            <span aria-hidden="true">{STEP_GLYPH[s.status]}</span>
            {s.label}
            {s.detail && s.status === "running" ? ` · ${s.detail}` : ""}
            <span className="sr-only">, {STEP_STATUS_WORD[s.status]}</span>
          </span>
        </li>
      ))}
    </ol>
  );
}

function QueuedDetail({ bot, now }: { bot: BotQueued; now: number }) {
  const start = parseTime(bot.startsAt);
  const end = parseTime(bot.finishesAt);
  let startText = "when a slot frees up";
  if (start != null) startText = start > now ? `in ${approxSpan(start - now)} · ${clockTime(bot.startsAt)}` : "now";
  return (
    <>
      <dl className="pr-facts">
        <Fact label="Position">{bot.position != null ? `#${bot.position} in queue` : "not listed in the queue"}</Fact>
        <Fact label="Can start" title={exactLocal(bot.startsAt) ?? undefined}>
          {startText}
        </Fact>
        {bot.reason && <Fact label="Waiting for">{bot.reason}</Fact>}
        {bot.priority && <Fact label="Priority">yes</Fact>}
        {end != null && (
          <Fact label="Estimated finish" title={exactLocal(bot.finishesAt) ?? undefined}>
            {end > now ? `${approxSpan(end - now)} · ${clockTime(bot.finishesAt)}` : "any time now"}
          </Fact>
        )}
      </dl>
      {end != null && <p className="pr-detail__note">{QUEUED_ESTIMATE_NOTE}, not the bot's own figure.</p>}
    </>
  );
}

function Reviewers({ pr }: { pr: PullRequest }) {
  const parts = [botReviewerText(pr.bot)];
  if (pr.humans.length === 0) parts.push("No human reviewers requested");
  else parts.push(...pr.humans.map(humanStateText));
  return <>{parts.join(" · ")}</>;
}

function Actions({ pr, now }: { pr: PullRequest; now: number }) {
  const openUrl = useApp((s) => s.openUrl);
  const followPr = useApp((s) => s.followPr);
  const unfollowPr = useApp((s) => s.unfollowPr);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!message) return;
    const t = setTimeout(() => setMessage(null), 4000);
    return () => clearTimeout(t);
  }, [message]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setMessage(null);
    try {
      await fn();
    } catch (e) {
      setMessage(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(pr.url);
      setMessage("Link copied");
    } catch {
      setMessage("Could not copy the link");
    }
  };

  return (
    <div className="pr-actions">
      <button type="button" className="btn" onClick={() => void openUrl(pr.url)}>
        Open on GitHub
      </button>
      {!pr.isMine &&
        (pr.isFollowed ? (
          <button type="button" className="btn" disabled={busy} onClick={() => void run(() => unfollowPr(pr.id))}>
            Unfollow
          </button>
        ) : (
          <button type="button" className="btn" disabled={busy} onClick={() => void run(() => followPr(pr.id))}>
            Follow
          </button>
        ))}
      <button type="button" className="btn ghost" onClick={() => void copy()}>
        Copy link
      </button>
      <RunButtons pr={pr} />
      <SelfReviewAction pr={pr} now={now} />
      <span className="pr-actions__msg dim" role="status">
        {message}
      </span>
      <MergeAction pr={pr} />
    </div>
  );
}

/**
 * "Squash and merge": opens the confirmation when GitHub says the PR is
 * ready; otherwise stays in the tab order, does nothing, and shows why.
 */
function MergeAction({ pr }: { pr: PullRequest }) {
  const [open, setOpen] = useState(false);
  const reasonId = useId();
  if (!showMergeButton(pr)) return null;
  const ready = pr.merge.ready;
  return (
    <span className="pr-merge">
      <button
        type="button"
        className="btn pr-merge__btn"
        aria-disabled={ready ? undefined : true}
        aria-describedby={ready ? undefined : reasonId}
        onClick={() => {
          if (ready) setOpen(true);
        }}
      >
        Squash and merge
      </button>
      {!ready && (
        <span id={reasonId} className="pr-merge__reason dim">
          {pr.merge.reason ?? "Not ready to merge"}
        </span>
      )}
      {open && <MergeDialog pr={pr} onClose={() => setOpen(false)} />}
    </span>
  );
}

/**
 * "Post /self-reviewed": opens the confirmation. Emphasised when the gate asks
 * for it, plain when it could be posted but is not asked for, and "Post again"
 * beside a short status once the app has posted it.
 */
function SelfReviewAction({ pr, now }: { pr: PullRequest; now: number }) {
  const [open, setOpen] = useState(false);
  const { state, postedAt } = pr.selfReview;
  const label = selfReviewButtonLabel(state);
  if (label == null) return null;
  return (
    <span className="pr-self-review">
      {state === "posted" && (
        <span className="pr-self-review__status" title={exactLocal(postedAt) ?? undefined}>
          {postedText(postedAt, now)}
        </span>
      )}
      {/* One button in one place for every state, so focus returns to it after posting. */}
      <button
        type="button"
        className={state === "wanted" ? "btn primary" : "btn"}
        onClick={() => setOpen(true)}
      >
        {state === "posted" ? (
          label
        ) : (
          <>
            Post <span className="mono">{SELF_REVIEW_COMMAND}</span>
          </>
        )}
      </button>
      {open && <SelfReviewDialog pr={pr} onClose={() => setOpen(false)} />}
    </span>
  );
}
