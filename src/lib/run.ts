// Pure helpers for running a project's example apps: labels, durations,
// which platforms a PR's project offers, and the run dialog's defaults.

import type {
  RunDevice,
  RunJob,
  RunPlatform,
  RunPlatformOptions,
  RunProject,
  RunSource,
  RunState,
  RunStep,
  StepStatus,
} from "../types";
import type { RunPrefs } from "../state/store";
import { exactSpan, parseTime } from "./format";

/** Display order of the platforms, everywhere. */
export const PLATFORMS: RunPlatform[] = ["ios", "android"];

export const PLATFORM_LABEL: Record<RunPlatform, string> = { ios: "iOS", android: "Android" };

/** The example apps the PR's project offers, iOS first. Empty for most projects. */
export function platformsFor(targets: RunProject[], repo: string): RunPlatform[] {
  const offered = targets.find((t) => t.repo === repo)?.platforms ?? [];
  return PLATFORMS.filter((p) => offered.includes(p));
}

/** `owner/name#123` -> 123. NaN when the id has no number. */
export function prNumberOf(prId: string): number {
  return Number(prId.slice(prId.lastIndexOf("#") + 1));
}

/** The short form of a commit, as GitHub shows it. */
export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

/**
 * A path fit for one line: the home directory as `~`, and when still longer
 * than `max`, the middle elided, keeping the start and the last folders.
 */
export function shortenPath(path: string, max = 56): string {
  let p = path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~");
  if (p.length <= max) return p;
  const parts = p.split("/");
  // Keep the first part ("~" or "") and as many trailing folders as fit.
  const head = parts[0] === "" ? `/${parts[1] ?? ""}` : parts[0];
  const tail: string[] = [];
  for (let i = parts.length - 1; i > 1; i--) {
    const next = [parts[i], ...tail].join("/");
    if (head.length + 3 + next.length > max && tail.length > 0) break;
    tail.unshift(parts[i]);
  }
  p = `${head}/…/${tail.join("/")}`;
  return p;
}

/** A stable key for a source in the radio list. */
export function sourceKey(source: RunSource): string {
  return `${source.kind}:${source.path}`;
}

/** The kind remembered in prefs, when a source of that kind is offered; otherwise the first source. */
export function defaultSource(sources: RunSource[], prefs: RunPrefs): RunSource | null {
  if (prefs.sourceKind) {
    const match = sources.find((s) => s.kind === prefs.sourceKind);
    if (match) return match;
  }
  return sources[0] ?? null;
}

/** The device remembered for the platform, when it is still offered; otherwise the first device. */
export function defaultDevice(options: RunPlatformOptions | undefined, prefs: RunPrefs): RunDevice | null {
  if (!options) return null;
  const remembered = prefs.devices[options.platform];
  return options.devices.find((d) => d.id === remembered) ?? options.devices[0] ?? null;
}

/** One line for a device in the select: "iPhone 17 · iOS 27.2 · Running". */
export function deviceLabel(device: RunDevice): string {
  const parts = [device.name];
  if (device.detail) parts.push(device.detail);
  if (device.running) parts.push("Running");
  return parts.join(" · ");
}

export const RUN_STATE_TEXT: Record<RunState, string> = {
  running: "Running",
  succeeded: "Opened",
  failed: "Failed",
  cancelled: "Cancelled",
};

export const STEP_STATUS_TEXT: Record<StepStatus, string> = {
  done: "Done",
  running: "Running",
  pending: "Not started",
  failed: "Failed",
};

/** "iOS · #5736 · iPhone 17". */
export function jobHeading(job: RunJob): string {
  return `${PLATFORM_LABEL[job.platform]} · #${prNumberOf(job.prId)} · ${job.deviceName}`;
}

/** The step running now, if any. */
export function currentStep(job: RunJob): RunStep | null {
  return job.steps.find((s) => s.status === "running") ?? null;
}

/** The compact status in the header: "Running on iOS · Build the SDK", "Opened on iPhone 17", "iOS run failed". */
export function headerStatusText(job: RunJob): string {
  const platform = PLATFORM_LABEL[job.platform];
  switch (job.state) {
    case "running": {
      const step = currentStep(job);
      return step ? `Running on ${platform} · ${step.label}` : `Running on ${platform}`;
    }
    case "succeeded":
      return `Opened on ${job.deviceName}`;
    case "failed":
      return `${platform} run failed`;
    case "cancelled":
      return `${platform} run cancelled`;
  }
}

/** The marker on the row of the PR being run, while it runs: "Running on iOS". null otherwise. */
export function rowMarkerText(job: RunJob | null, prId: string): string | null {
  if (!job || job.state !== "running" || job.prId !== prId) return null;
  return `Running on ${PLATFORM_LABEL[job.platform]}`;
}

/** Milliseconds from `start` to `end`, or to `now` while `end` is unknown. null without a start. */
export function spanMs(start: string | null, end: string | null, now: number): number | null {
  const s = parseTime(start);
  if (s == null) return null;
  const e = end == null ? now : parseTime(end);
  if (e == null) return null;
  return Math.max(0, e - s);
}

/** "42 s", "3 min 5 s", "1 h 2 min". */
export function durationText(ms: number): string {
  return exactSpan(ms / 1000);
}

/** How long the job has run, or ran. */
export function jobElapsedText(job: RunJob, now: number): string | null {
  const ms = spanMs(job.startedAt, job.finishedAt, now);
  return ms == null ? null : durationText(ms);
}

/** How long a step took, or has taken so far while running. null when it has not started. */
export function stepDurationText(step: RunStep, now: number): string | null {
  if (step.status === "pending") return null;
  if (step.status === "running") {
    const ms = spanMs(step.startedAt, null, now);
    return ms == null ? null : durationText(ms);
  }
  if (!step.finishedAt) return null;
  const ms = spanMs(step.startedAt, step.finishedAt, now);
  return ms == null ? null : durationText(ms);
}

/** True while some run is in progress, whatever PR it is for. */
export function isRunning(job: RunJob | null): boolean {
  return job?.state === "running";
}
