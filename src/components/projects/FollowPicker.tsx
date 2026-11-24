// Browse a project's open pull requests and follow or unfollow them; or
// follow any PR directly by its link.
import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";
import { errorMessage, useApp } from "../../state/store";
import type { PrSummary } from "../../types";
import { Dialog } from "../ui/Dialog";
import { formatAgo } from "../shell/time";
import "./FollowPicker.css";

type ListState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; prs: PrSummary[] };

export function FollowPicker({ repo, onClose }: { repo: string; onClose(): void }) {
  const listOpenPrs = useApp((s) => s.listOpenPrs);
  const followPr = useApp((s) => s.followPr);
  const unfollowPr = useApp((s) => s.unfollowPr);
  const followed = useApp((s) => s.config?.followed);

  const [list, setList] = useState<ListState>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [filter, setFilter] = useState("");
  const [link, setLink] = useState("");
  const [linkBusy, setLinkBusy] = useState(false);
  const [linkMsg, setLinkMsg] = useState<{ kind: "error" | "ok"; text: string } | null>(null);
  const [pending, setPending] = useState<Set<string>>(() => new Set());
  const [rowError, setRowError] = useState<string | null>(null);
  const linkInput = useRef<HTMLInputElement>(null);
  const linkId = useId();
  const linkMsgId = useId();
  const filterId = useId();

  useEffect(() => {
    let live = true;
    setList({ status: "loading" });
    listOpenPrs(repo).then(
      (prs) => live && setList({ status: "ready", prs }),
      (e) => live && setList({ status: "error", message: errorMessage(e) }),
    );
    return () => {
      live = false;
    };
  }, [repo, listOpenPrs, attempt]);

  const followedSet = useMemo(() => new Set(followed ?? []), [followed]);
  const isFollowed = (pr: PrSummary) => (followed ? followedSet.has(pr.id) : pr.isFollowed);

  const visible = useMemo(() => {
    if (list.status !== "ready") return [];
    const words = filter.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (words.length === 0) return list.prs;
    return list.prs.filter((pr) => {
      const hay = `#${pr.number} ${pr.title} ${pr.author}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  }, [list, filter]);

  const followLink = async (e: FormEvent) => {
    e.preventDefault();
    const input = link.trim();
    if (!input || linkBusy) return;
    setLinkBusy(true);
    setLinkMsg(null);
    try {
      await followPr(input);
      setLink("");
      setLinkMsg({ kind: "ok", text: `Following ${input}.` });
    } catch (err) {
      setLinkMsg({ kind: "error", text: errorMessage(err) });
      linkInput.current?.focus();
    } finally {
      setLinkBusy(false);
    }
  };

  const toggle = async (pr: PrSummary) => {
    const on = isFollowed(pr);
    setPending((p) => new Set(p).add(pr.id));
    setRowError(null);
    try {
      if (on) await unfollowPr(pr.id);
      else await followPr(pr.id);
    } catch (err) {
      setRowError(`Could not ${on ? "unfollow" : "follow"} #${pr.number}: ${errorMessage(err)}`);
    } finally {
      setPending((p) => {
        const next = new Set(p);
        next.delete(pr.id);
        return next;
      });
    }
  };

  return (
    <Dialog
      title={`Follow pull requests in ${repo}`}
      description="Followed pull requests appear under Following with their review status."
      onClose={onClose}
      initialFocus={linkInput}
      size="lg"
    >
      <form className="dialog-body follow-link" onSubmit={followLink}>
        <label className="dialog-label" htmlFor={linkId}>
          Follow by link
        </label>
        <div className="follow-link-row">
          <input
            ref={linkInput}
            id={linkId}
            className="input mono"
            placeholder="https://github.com/owner/name/pull/123 or owner/name#123"
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={link}
            onChange={(e) => {
              setLink(e.target.value);
              if (linkMsg) setLinkMsg(null);
            }}
            aria-invalid={linkMsg?.kind === "error" ? true : undefined}
            aria-describedby={linkMsg ? linkMsgId : undefined}
          />
          <button type="submit" className="btn primary" disabled={linkBusy || !link.trim()} aria-busy={linkBusy || undefined}>
            {linkBusy ? "Following…" : "Follow"}
          </button>
        </div>
        {linkMsg && (
          <p
            id={linkMsgId}
            className={linkMsg.kind === "error" ? "dialog-error" : "follow-ok"}
            role={linkMsg.kind === "error" ? "alert" : "status"}
          >
            {linkMsg.text}
          </p>
        )}
      </form>

      <div className="follow-list-head">
        <label className="sr-only" htmlFor={filterId}>
          Filter open pull requests
        </label>
        <input
          id={filterId}
          className="input"
          type="search"
          placeholder="Filter by title, number or author"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          disabled={list.status !== "ready"}
        />
        {list.status === "ready" && (
          <span className="dim follow-count" aria-live="polite">
            {visible.length === list.prs.length ? `${list.prs.length} open` : `${visible.length} of ${list.prs.length}`}
          </span>
        )}
      </div>

      {rowError && (
        <p className="dialog-error follow-row-error" role="alert">
          {rowError}
        </p>
      )}

      <div className="follow-list" aria-busy={list.status === "loading"}>
        {list.status === "loading" && <p className="follow-state dim">Loading open pull requests…</p>}
        {list.status === "error" && (
          <div className="follow-state">
            <p className="dialog-error" role="alert">
              Could not load open pull requests: {list.message}
            </p>
            <button type="button" className="btn" onClick={() => setAttempt((n) => n + 1)}>
              Try again
            </button>
          </div>
        )}
        {list.status === "ready" && list.prs.length === 0 && (
          <p className="follow-state dim">No open pull requests</p>
        )}
        {list.status === "ready" && list.prs.length > 0 && visible.length === 0 && (
          <p className="follow-state dim">No open pull requests match “{filter.trim()}”</p>
        )}
        {visible.length > 0 && (
          <ul className="follow-rows" aria-label={`Open pull requests in ${repo}`}>
            {visible.map((pr) => {
              const on = isFollowed(pr);
              const busy = pending.has(pr.id);
              return (
                <li key={pr.id} className="follow-row">
                  <div className="follow-main">
                    <div className="follow-title" title={pr.title}>
                      <span className="mono dim">#{pr.number}</span> {pr.title}
                    </div>
                    <div className="follow-sub">
                      <span className="follow-author">{pr.author}</span>
                      {pr.isDraft && <span className="pill draft">Draft</span>}
                      <span>updated {formatAgo(pr.updatedAt)}</span>
                    </div>
                  </div>
                  {pr.isMine ? (
                    <span className="follow-yours dim">Yours</span>
                  ) : (
                    <button
                      type="button"
                      className={`btn follow-toggle${on ? " on" : ""}`}
                      aria-pressed={on}
                      aria-label={`Follow #${pr.number} ${pr.title}`}
                      disabled={busy}
                      onClick={() => toggle(pr)}
                    >
                      {on ? "Following" : "Follow"}
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      <div className="dialog-actions">
        <button type="button" className="btn" onClick={onClose}>
          Done
        </button>
      </div>
    </Dialog>
  );
}
