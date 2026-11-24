// "Run on iOS" / "Run on Android" in a PR's detail: one button per example
// app the PR's project offers, and nothing at all when it offers none.
import { useState } from "react";
import type { PullRequest, RunPlatform } from "../../types";
import { useApp } from "../../state/store";
import { PLATFORM_LABEL, platformsFor } from "../../lib/run";
import { RunDialog } from "./RunDialog";
import { RunIcon } from "./RunIcon";
import "./RunButtons.css";

export function RunButtons({ pr }: { pr: PullRequest }) {
  const runTargets = useApp((s) => s.runTargets);
  const [open, setOpen] = useState<RunPlatform | null>(null);
  const platforms = platformsFor(runTargets, pr.repo);
  if (platforms.length === 0) return null;
  return (
    <>
      {platforms.map((p) => (
        <button key={p} type="button" className="btn run-btn" onClick={() => setOpen(p)}>
          <RunIcon platform={p} />
          Run on {PLATFORM_LABEL[p]}
        </button>
      ))}
      {open && <RunDialog pr={pr} platform={open} onClose={() => setOpen(null)} />}
    </>
  );
}
