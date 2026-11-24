// The ⌘K palette: one input that finds pull requests across every project
// and a few commands (go to a view, show a tracked contributor's pull
// requests, track someone, refresh, run the open PR's example app, post the
// open PR's self-review).
// Renders nothing while closed, except the run dialog it may have opened.

import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { useApp } from "../../state/store";
import type { PullRequest, RunPlatform } from "../../types";
import { allPrs, isListed, matchesQuery, matchesView, sameLogin, sameView, type View } from "../../lib/select";
import { botStatusText, matchesWords } from "../../lib/format";
import { milestoneKey } from "../../lib/milestones";
import { areaVisible, titleAreas } from "../../lib/areas";
import { createdRecently } from "../../lib/filters";
import { hasMod } from "../../lib/keys";
import { PLATFORM_LABEL, platformsFor } from "../../lib/run";
import { selfReviewPaletteLabel } from "../../lib/selfReview";
import { RunDialog } from "../run/RunDialog";
import { CommandText, SelfReviewDialog } from "../table/SelfReviewDialog";
import { TrackContributorDialog } from "../people/TrackContributorDialog";
import { possessive } from "../../lib/people";
import "./CommandPalette.css";

const MAX_PRS = 50;

type Item =
  | { kind: "command"; key: string; label: string; hint: string; run(): void }
  | { kind: "pr"; key: string; pr: PullRequest };

/** Opens and closes the palette on ⌘K (Ctrl K off macOS), from anywhere. */
function usePaletteShortcut() {
  const paletteOpen = useApp((s) => s.paletteOpen);
  const setPaletteOpen = useApp((s) => s.setPaletteOpen);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== "k" || !hasMod(e) || e.altKey || e.shiftKey) return;
      e.preventDefault();
      setPaletteOpen(!paletteOpen);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paletteOpen, setPaletteOpen]);
}

interface RunRequestState {
  pr: PullRequest;
  platform: RunPlatform;
}

export function CommandPalette() {
  usePaletteShortcut();
  const open = useApp((s) => s.paletteOpen);
  // The run dialog outlives the palette that opened it.
  const [run, setRun] = useState<RunRequestState | null>(null);
  const [selfReview, setSelfReview] = useState<PullRequest | null>(null);
  const [tracking, setTracking] = useState(false);
  return (
    <>
      {open && (
        <Palette
          onRun={(pr, platform) => setRun({ pr, platform })}
          onSelfReview={setSelfReview}
          onTrack={() => setTracking(true)}
        />
      )}
      {run && <RunDialog pr={run.pr} platform={run.platform} onClose={() => setRun(null)} />}
      {selfReview && <SelfReviewDialog pr={selfReview} onClose={() => setSelfReview(null)} />}
      {tracking && <TrackContributorDialog onClose={() => setTracking(false)} />}
    </>
  );
}

interface PaletteProps {
  onRun(pr: PullRequest, platform: RunPlatform): void;
  onSelfReview(pr: PullRequest): void;
  /** Opens the "Track contributor" dialog. */
  onTrack(): void;
}

function Palette({ onRun, onSelfReview, onTrack }: PaletteProps) {
  const snapshot = useApp((s) => s.snapshot);
  const config = useApp((s) => s.config);
  const view = useApp((s) => s.view);
  const expandedId = useApp((s) => s.expandedId);
  const setView = useApp((s) => s.setView);
  const setQuery = useApp((s) => s.setQuery);
  const toggleExpanded = useApp((s) => s.toggleExpanded);
  const hiddenMilestones = useApp((s) => s.hiddenMilestones);
  const toggleMilestone = useApp((s) => s.toggleMilestone);
  const hiddenAreas = useApp((s) => s.hiddenAreas);
  const toggleArea = useApp((s) => s.toggleArea);
  const showDrafts = useApp((s) => s.showDrafts);
  const setShowDrafts = useApp((s) => s.setShowDrafts);
  const recentOnly = useApp((s) => s.recentOnly);
  const setRecentOnly = useApp((s) => s.setRecentOnly);
  const setFocused = useApp((s) => s.setFocused);
  const setPaletteOpen = useApp((s) => s.setPaletteOpen);
  const refresh = useApp((s) => s.refresh);
  const focusedId = useApp((s) => s.focusedId);
  const runTargets = useApp((s) => s.runTargets);

  const [text, setText] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  // Where focus was before the palette opened, to put it back on dismiss.
  const returnFocus = useRef<HTMLElement | null>(
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const dismiss = () => {
    setPaletteOpen(false);
    const el = returnFocus.current;
    if (el && el.isConnected) el.focus({ preventScroll: true });
  };

  const goTo = (target: View) => {
    if (!sameView(view, target)) setView(target);
    dismiss();
  };

  const repos = snapshot?.projects.map((p) => p.repo) ?? config?.projects ?? [];

  // "Run #5736 on iOS" for the PR that is open, or else the one the cursor is on.
  const targetId = expandedId ?? focusedId;
  const target = targetId ? allPrs(snapshot).find((p) => p.id === targetId) : undefined;
  const runCommands: Item[] = target
    ? platformsFor(runTargets, target.repo).map<Item>((platform) => ({
        kind: "command",
        key: `run:${platform}`,
        label: `Run #${target.number} on ${PLATFORM_LABEL[platform]}`,
        hint: "Example app",
        run: () => {
          dismiss();
          onRun(target, platform);
        },
      }))
    : [];

  // "Post /self-reviewed on #6038" for the same PR, through the same confirmation.
  const selfReviewLabel = target ? selfReviewPaletteLabel(target) : null;
  const selfReviewCommands: Item[] =
    target && selfReviewLabel
      ? [
          {
            kind: "command",
            key: "self-review",
            label: selfReviewLabel,
            hint: "Comment",
            run: () => {
              dismiss();
              onSelfReview(target);
            },
          },
        ]
      : [];

  const commands: Item[] = [
    ...selfReviewCommands,
    ...runCommands,
    { kind: "command", key: "go:mine", label: "Go to Mine", hint: "View", run: () => goTo({ kind: "mine" }) },
    { kind: "command", key: "go:following", label: "Go to Following", hint: "View", run: () => goTo({ kind: "following" }) },
    { kind: "command", key: "go:needsMe", label: "Go to Needs me", hint: "View", run: () => goTo({ kind: "needsMe" }) },
    ...repos.map<Item>((repo) => ({
      kind: "command",
      key: `go:${repo}`,
      label: `Go to ${repo}`,
      hint: "Project",
      run: () => goTo({ kind: "project", repo }),
    })),
    { kind: "command", key: "show:mine", label: "Show mine", hint: "Person", run: () => goTo({ kind: "mine" }) },
    ...(config?.contributors ?? []).map<Item>((login) => ({
      kind: "command",
      key: `show:${login}`,
      label: `Show ${possessive(login)} pull requests`,
      hint: "Person",
      run: () => goTo({ kind: "contributor", login }),
    })),
    {
      kind: "command",
      key: "track",
      label: "Track contributor…",
      hint: "Action",
      run: () => {
        dismiss();
        onTrack();
      },
    },
    {
      kind: "command",
      key: "refresh",
      label: "Refresh",
      hint: "Action",
      run: () => {
        dismiss();
        void refresh();
      },
    },
  ];

  const matchedCommands = commands.filter((c) => c.kind === "command" && matchesWords(c.label, text));
  const prs = useMemo(
    () => allPrs(snapshot).filter((pr) => matchesQuery(pr, text)).slice(0, MAX_PRS),
    [snapshot, text],
  );
  const prItems: Item[] = prs.map((pr) => ({ kind: "pr", key: `pr:${pr.id}`, pr }));
  // With nothing typed, commands lead; once typing, pull requests do.
  const items = text.trim() ? [...prItems, ...matchedCommands] : [...matchedCommands, ...prItems];
  const activeIndex = items.length ? Math.min(active, items.length - 1) : -1;

  useEffect(() => {
    setActive(0);
  }, [text]);

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(`[data-index="${activeIndex}"]`);
    el?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex]);

  const choosePr = (pr: PullRequest) => {
    const switching = !matchesView(pr, view);
    // A tracked contributor's PR is listed only in that person's view.
    if (switching) {
      const login = config?.contributors.find((c) => sameLogin(c, pr.author)) ?? pr.author;
      setView(isListed(pr) ? { kind: "project", repo: pr.repo } : { kind: "contributor", login });
    }
    setQuery("");
    // The palette searches every PR; if this one is a draft while drafts are
    // hidden, older than 3 days while only recent ones are listed, or its
    // milestone or areas are switched off, switch them back on so the jump
    // lands on a visible row. Only the areas this PR names come back, in its
    // own project.
    if (pr.isDraft && !showDrafts) setShowDrafts(true);
    if (recentOnly && !createdRecently(pr, Date.now())) setRecentOnly(false);
    if (hiddenMilestones.includes(milestoneKey(pr))) toggleMilestone(milestoneKey(pr));
    if (!areaVisible(pr, hiddenAreas)) {
      const off = hiddenAreas[pr.repo] ?? [];
      for (const area of titleAreas(pr.title)) if (off.includes(area)) toggleArea(pr.repo, area);
    }
    if (switching || expandedId !== pr.id) toggleExpanded(pr.id);
    else setFocused(pr.id);
    setPaletteOpen(false);
    // The table moves focus to the row when focus or expansion changes; if
    // neither did, put focus on the row here.
    setTimeout(() => {
      if (document.activeElement && document.activeElement !== document.body) return;
      const row = [...document.querySelectorAll<HTMLElement>("[data-pr-row]")].find(
        (el) => el.dataset.prRow === pr.id,
      );
      row?.focus({ preventScroll: true });
      row?.scrollIntoView?.({ block: "nearest" });
    }, 0);
  };

  const run = (item: Item | undefined) => {
    if (!item) return;
    if (item.kind === "pr") choosePr(item.pr);
    else item.run();
  };

  const onKeyDown = (e: ReactKeyboardEvent) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (items.length) setActive((activeIndex + 1) % items.length);
        break;
      case "ArrowUp":
        e.preventDefault();
        if (items.length) setActive((activeIndex - 1 + items.length) % items.length);
        break;
      case "Enter":
        e.preventDefault();
        run(items[activeIndex]);
        break;
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        dismiss();
        break;
      case "Tab":
        // Focus stays in the input; the list is driven from it.
        e.preventDefault();
        break;
    }
  };

  const optionId = (i: number) => `palette-option-${i}`;
  const renderItem = (item: Item, i: number) => (
    <div
      key={item.key}
      id={optionId(i)}
      data-index={i}
      role="option"
      aria-selected={i === activeIndex}
      className={`palette__option${i === activeIndex ? " active" : ""}`}
      onMouseMove={() => i !== activeIndex && setActive(i)}
      onMouseDown={(e) => e.preventDefault()}
      onClick={() => run(item)}
    >
      {item.kind === "command" ? (
        <>
          <span className="palette__label">
            <CommandText text={item.label} />
          </span>
          <span className="palette__hint dim">{item.hint}</span>
        </>
      ) : (
        <>
          <span className="palette__pr">
            <span className="palette__label">{item.pr.title}</span>
            <span className="palette__sub dim">
              {item.pr.repo} <span className="mono">#{item.pr.number}</span>
              {!item.pr.isMine && ` · ${item.pr.author}`}
            </span>
          </span>
          <span className="palette__hint dim">{botStatusText(item.pr.bot)}</span>
        </>
      )}
    </div>
  );

  const prStart = text.trim() ? 0 : matchedCommands.length;
  const cmdStart = text.trim() ? prItems.length : 0;
  const prSection = prItems.length > 0 && (
    <div role="group" aria-labelledby="palette-group-prs" key="prs">
      <div className="palette__group" id="palette-group-prs">
        Pull requests
      </div>
      {prItems.map((item, i) => renderItem(item, prStart + i))}
    </div>
  );
  const cmdSection = matchedCommands.length > 0 && (
    <div role="group" aria-labelledby="palette-group-commands" key="commands">
      <div className="palette__group" id="palette-group-commands">
        Commands
      </div>
      {matchedCommands.map((item, i) => renderItem(item, cmdStart + i))}
    </div>
  );

  return (
    <div
      className="palette-backdrop"
      onMouseDown={(e) => {
        if (e.target !== e.currentTarget) return;
        // Stop the click's own focus change from undoing the focus we restore.
        e.preventDefault();
        dismiss();
      }}
    >
      <div className="palette" role="dialog" aria-modal="true" aria-label="Go to a pull request or command">
        <input
          ref={inputRef}
          className="palette__input"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-listbox"
          aria-autocomplete="list"
          aria-activedescendant={activeIndex >= 0 ? optionId(activeIndex) : undefined}
          aria-label="Search pull requests and commands"
          placeholder="Search pull requests and commands"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          spellCheck={false}
          autoComplete="off"
        />
        <div className="palette__list" id="palette-listbox" role="listbox" aria-label="Results" ref={listRef}>
          {text.trim() ? [prSection, cmdSection] : [cmdSection, prSection]}
          {items.length === 0 && <div className="palette__empty dim">Nothing matches “{text.trim()}”</div>}
        </div>
        <div className="palette__foot dim" aria-hidden="true">
          <span>
            <span className="kbd">↑</span> <span className="kbd">↓</span> move
          </span>
          <span>
            <span className="kbd">↵</span> open
          </span>
          <span>
            <span className="kbd">esc</span> close
          </span>
        </div>
      </div>
    </div>
  );
}
