// Small line icons for running an example app. Decorative: the text beside
// them always names the platform.
import type { RunPlatform } from "../../types";

/** A phone with a play mark: "build and open on a device". The iOS one has an island, the Android one a camera dot. */
export function RunIcon({ platform }: { platform: RunPlatform }) {
  return (
    <svg className="run-icon" viewBox="0 0 14 14" width="13" height="13" aria-hidden="true" focusable="false">
      <rect x="3" y="0.75" width="8" height="12.5" rx="1.8" fill="none" stroke="currentColor" strokeWidth="1.2" />
      {platform === "ios" ? (
        <rect x="5.6" y="2.1" width="2.8" height="0.9" rx="0.45" fill="currentColor" />
      ) : (
        <circle cx="7" cy="2.6" r="0.6" fill="currentColor" />
      )}
      <path d="M6 5.6 L9 7.4 L6 9.2 Z" fill="currentColor" />
    </svg>
  );
}
