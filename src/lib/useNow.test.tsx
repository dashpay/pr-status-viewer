import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { useNow } from "./useNow";

let renders = 0;
function Clock({ fast }: { fast: boolean }) {
  const now = useNow(fast);
  renders++;
  return <span data-testid="now">{now}</span>;
}

describe("useNow", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    renders = 0;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("ticks every 30 seconds by default", () => {
    const { getByTestId } = render(<Clock fast={false} />);
    const first = getByTestId("now").textContent;
    act(() => vi.advanceTimersByTime(29_000));
    expect(getByTestId("now").textContent).toBe(first);
    act(() => vi.advanceTimersByTime(1_000));
    expect(getByTestId("now").textContent).not.toBe(first);
  });

  it("ticks every second while a caller wants fast ticks, and slows down after", () => {
    const { getByTestId, rerender } = render(<Clock fast={true} />);
    const a = getByTestId("now").textContent;
    act(() => vi.advanceTimersByTime(1_000));
    const b = getByTestId("now").textContent;
    expect(b).not.toBe(a);

    rerender(<Clock fast={false} />);
    const c = getByTestId("now").textContent;
    act(() => vi.advanceTimersByTime(5_000));
    expect(getByTestId("now").textContent).toBe(c);
  });

  it("shares one tick between callers", () => {
    const { getAllByTestId } = render(
      <>
        <Clock fast={false} />
        <Clock fast={true} />
      </>,
    );
    act(() => vi.advanceTimersByTime(1_000));
    const [x, y] = getAllByTestId("now").map((e) => e.textContent);
    expect(x).toBe(y);
    expect(renders).toBeGreaterThan(2);
  });
});
