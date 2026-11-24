// One shared clock for every live time on screen. Components ask for `now`
// with useNow(fast); the single timer ticks every second while any mounted
// caller wants fast ticks, otherwise every 30 seconds. All callers receive the
// same timestamp on each tick, so countdowns move together.

import { useEffect, useState } from "react";

export const FAST_INTERVAL_MS = 1_000;
export const SLOW_INTERVAL_MS = 30_000;

interface Subscriber {
  fast: boolean;
  set: (now: number) => void;
}

const subscribers = new Set<Subscriber>();
let timer: ReturnType<typeof setInterval> | null = null;
let timerFast = false;

function tick() {
  const now = Date.now();
  for (const s of subscribers) s.set(now);
}

function reschedule() {
  const fast = [...subscribers].some((s) => s.fast);
  if (subscribers.size === 0) {
    if (timer) clearInterval(timer);
    timer = null;
    return;
  }
  if (timer && fast === timerFast) return;
  if (timer) clearInterval(timer);
  timerFast = fast;
  timer = setInterval(tick, fast ? FAST_INTERVAL_MS : SLOW_INTERVAL_MS);
}

/** The current time in milliseconds, re-rendering on the shared tick. */
export function useNow(fast = false): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const sub: Subscriber = { fast, set: setNow };
    subscribers.add(sub);
    reschedule();
    // Catch up immediately when switching rate, so a countdown never lags a full slow tick.
    setNow(Date.now());
    return () => {
      subscribers.delete(sub);
      reschedule();
    };
  }, [fast]);
  return now;
}
