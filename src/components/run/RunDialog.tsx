// The run dialog: choose the platform (when the project has both apps), where
// the code comes from and the device, then "Build and run". Asks the backend
// for the choices fresh every time it opens.
import { useEffect, useId, useRef, useState } from "react";
import type { PullRequest, RunDevice, RunOptions, RunPlatform, RunSource } from "../../types";
import { errorMessage, useApp } from "../../state/store";
import {
  PLATFORM_LABEL,
  defaultDevice,
  defaultSource,
  deviceLabel,
  isRunning,
  shortSha,
  shortenPath,
  sourceKey,
} from "../../lib/run";
import { t3Working } from "../../lib/merge";
import { Dialog } from "../ui/Dialog";
import "./RunDialog.css";

interface RunDialogProps {
  pr: PullRequest;
  /** The platform the user asked for; the dialog starts on it. */
  platform: RunPlatform;
  onClose(): void;
}

export const T3_CAUTION = "T3 Code is working on this pull request. Building its worktree now may collide with that work.";
export const FIRST_BUILD_NOTE =
  "The first build can take a long time. Running replaces the app on the device but keeps its data.";

export function RunDialog({ pr, platform: initialPlatform, onClose }: RunDialogProps) {
  const runOptions = useApp((s) => s.runOptions);
  const runStart = useApp((s) => s.runStart);
  const runPrefs = useApp((s) => s.runPrefs);
  const otherRunning = useApp((s) => isRunning(s.runJob));
  const setRunPanelOpen = useApp((s) => s.setRunPanelOpen);

  const [options, setOptions] = useState<RunOptions | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [platform, setPlatform] = useState<RunPlatform>(initialPlatform);
  const [sourceId, setSourceId] = useState<string | null>(null);
  const [devices, setDevices] = useState<Partial<Record<RunPlatform, string>>>({});
  const [busy, setBusy] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);
  const deviceSelectId = useId();
  const busyNoteId = useId();

  useEffect(() => {
    let live = true;
    runOptions(pr.id).then(
      (opts) => {
        if (!live) return;
        setOptions(opts);
        const src = defaultSource(opts.sources, runPrefs);
        setSourceId(src ? sourceKey(src) : null);
        const chosen: Partial<Record<RunPlatform, string>> = {};
        for (const p of opts.platforms) {
          const d = defaultDevice(p, runPrefs);
          if (d) chosen[p.platform] = d.id;
        }
        setDevices(chosen);
        if (!opts.platforms.some((p) => p.platform === initialPlatform) && opts.platforms[0]) {
          setPlatform(opts.platforms[0].platform);
        }
      },
      (e) => {
        if (live) setLoadError(errorMessage(e));
      },
    );
    return () => {
      live = false;
    };
    // Asked once per opening; prefs read at that moment.
  }, []);

  // Once the choices arrive, move focus from "Cancel" to the primary button,
  // unless the user has moved it.
  useEffect(() => {
    if (options && document.activeElement === cancelRef.current) primaryRef.current?.focus();
  }, [options]);

  const source = options?.sources.find((s) => sourceKey(s) === sourceId) ?? null;
  const platformOpts = options?.platforms.find((p) => p.platform === platform);
  const device: RunDevice | null = platformOpts?.devices.find((d) => d.id === devices[platform]) ?? null;
  const canRun = !!options && !!source && !!platformOpts && !platformOpts.unavailable && !!device && !otherRunning && !busy;

  const start = async () => {
    if (!canRun || !source || !device) return;
    setBusy(true);
    setStartError(null);
    try {
      await runStart({
        id: pr.id,
        platform,
        sourceKind: source.kind,
        sourcePath: source.kind === "worktree" ? source.path : null,
        deviceId: device.id,
      });
      onClose();
    } catch (e) {
      setStartError(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Dialog title={`Run #${pr.number}`} description={pr.title} onClose={onClose} size="md">
      <div className="dialog-body run-dialog">
        {!options && !loadError && (
          <p className="run-dialog__loading dim" role="status">
            <span className="spinner" aria-hidden="true" /> Looking for worktrees and devices…
          </p>
        )}
        {loadError && (
          <p className="dialog-error" role="alert">
            {loadError}
          </p>
        )}
        {options && (
          <>
            {options.platforms.length > 1 && (
              <div className="run-dialog__platforms seg" role="group" aria-label="Example app">
                {options.platforms.map((p) => (
                  <button
                    key={p.platform}
                    type="button"
                    aria-pressed={p.platform === platform}
                    onClick={() => setPlatform(p.platform)}
                  >
                    {PLATFORM_LABEL[p.platform]}
                  </button>
                ))}
              </div>
            )}

            <fieldset className="run-dialog__group">
              <legend className="dialog-label">Code</legend>
              <div className="run-sources">
                {options.sources.map((s) => (
                  <SourceOption
                    key={sourceKey(s)}
                    source={s}
                    checked={sourceKey(s) === sourceId}
                    onChoose={() => setSourceId(sourceKey(s))}
                  />
                ))}
              </div>
            </fieldset>

            <div className="run-dialog__group">
              <label className="dialog-label" htmlFor={deviceSelectId}>
                Device
              </label>
              {platformOpts?.unavailable ? (
                <p className="run-dialog__unavailable">{platformOpts.unavailable}</p>
              ) : platformOpts && platformOpts.devices.length > 0 ? (
                <select
                  id={deviceSelectId}
                  className="input run-dialog__select"
                  value={device?.id ?? ""}
                  onChange={(e) => setDevices((d) => ({ ...d, [platform]: e.target.value }))}
                >
                  {platformOpts.devices.map((d) => (
                    <option key={d.id} value={d.id}>
                      {deviceLabel(d)}
                    </option>
                  ))}
                </select>
              ) : (
                <p className="run-dialog__unavailable">
                  No {platform === "ios" ? "simulators" : "emulators or devices"} found.
                </p>
              )}
            </div>

            {source?.kind === "worktree" && t3Working(pr) && (
              <p className="run-dialog__caution">
                <span aria-hidden="true">⚠ </span>
                {T3_CAUTION}
              </p>
            )}
            <p className="run-dialog__note dim">{FIRST_BUILD_NOTE}</p>
            {startError && (
              <p className="dialog-error" role="alert">
                {startError}
              </p>
            )}
          </>
        )}
      </div>
      <div className="dialog-actions run-dialog__actions">
        {otherRunning && (
          <span className="run-dialog__busy">
            <span id={busyNoteId} className="dim">
              Another run is in progress
            </span>
            <button
              type="button"
              className="btn ghost"
              onClick={() => {
                setRunPanelOpen(true);
                onClose();
              }}
            >
              Show progress
            </button>
          </span>
        )}
        <button ref={cancelRef} type="button" className="btn" onClick={onClose}>
          Cancel
        </button>
        <button
          ref={primaryRef}
          type="button"
          className="btn primary"
          disabled={!canRun}
          aria-describedby={otherRunning ? busyNoteId : undefined}
          onClick={() => void start()}
        >
          {busy ? "Starting…" : "Build and run"}
        </button>
      </div>
    </Dialog>
  );
}

function SourceOption({ source, checked, onChoose }: { source: RunSource; checked: boolean; onChoose(): void }) {
  const isHead = source.kind === "prHead";
  return (
    <label className={`run-source${checked ? " on" : ""}`}>
      <input type="radio" name="run-source" checked={checked} onChange={onChoose} />
      <span className="run-source__text">
        <span className="run-source__title">
          {isHead ? "Pull request head" : "Your worktree"} ·{" "}
          <span className="mono">{isHead ? shortSha(source.sha) : (source.branch ?? `detached at ${shortSha(source.sha)}`)}</span>
        </span>
        {isHead ? (
          <span className="run-source__line dim">A clean copy that PR Status keeps for itself</span>
        ) : (
          <>
            <span className="run-source__line run-source__path dim mono" title={source.path}>
              {shortenPath(source.path)}
            </span>
            <span className="run-source__line">
              {source.atPrHead ? (
                <span className="run-source__ok">At the pull request head</span>
              ) : (
                <span className="run-source__warn">
                  Not at the pull request head: <span className="mono">{shortSha(source.sha)}</span>
                </span>
              )}
              {source.dirty && <span className="run-source__warn"> · Has uncommitted changes</span>}
            </span>
          </>
        )}
      </span>
    </label>
  );
}
