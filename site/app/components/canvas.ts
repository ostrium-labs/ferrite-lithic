/**
 * React bindings for the canvas demos.
 *
 * The demos are plain functions over a 2D context and a timestamp — they know nothing
 * about React, which is why they can be read and reasoned about on their own. This file is
 * the only place that knows about lifecycle.
 *
 * The lifecycle point that matters: an animation must not start until the canvas has a
 * size, and must stop when it scrolls away or unmounts. Getting the second half wrong is
 * how a documentation page ends up running six requestAnimationFrame loops in the
 * background, which is a real battery cost on a page someone may leave open while reading.
 */

import { useEffect, useRef } from "react";

import { logicalSize, onFrame, onVisibility, pause, prepareCanvas, resume } from "../lib/ticker";

/** A canvas driven by a draw function on the shared animation loop. */
export function useCanvas(
  draw: (context: CanvasRenderingContext2D, width: number, height: number, elapsed: number) => void,
): React.RefObject<HTMLCanvasElement | null> {
  const ref = useRef<HTMLCanvasElement | null>(null);
  // Held in a ref so changing the draw function does not restart the animation: the
  // callbacks below are registered once, and the latest draw function is read at frame time.
  const latest = useRef(draw);
  latest.current = draw;

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;

    const render = (elapsed: number): void => {
      const context = prepareCanvas(canvas);
      if (!context) return;
      const size = logicalSize(canvas);
      latest.current(context, size.width, size.height, elapsed);
    };

    onFrame(canvas, render);
    onVisibility(
      canvas,
      () => resume(canvas),
      () => pause(canvas),
    );

    const onResize = (): void => render(0);
    globalThis.addEventListener("resize", onResize);
    render(0);

    // Pause on unmount. `onFrame` holds the canvas in a set, so without this the loop
    // keeps a detached canvas alive and keeps drawing into it.
    return () => {
      pause(canvas);
      globalThis.removeEventListener("resize", onResize);
    };
  }, []);

  return ref;
}

/**
 * The hero timing diagram, which draws itself once and then holds.
 *
 * Not looped: a hero that never stops competing with the paragraph beneath it, and the
 * diagram has said everything it has to say by the time the last bus cell is filled.
 */
export function useOnceCanvas(
  duration: number,
  draw: (context: CanvasRenderingContext2D, width: number, height: number, elapsed: number) => void,
): React.RefObject<HTMLCanvasElement | null> {
  const ref = useRef<HTMLCanvasElement | null>(null);
  const latest = useRef(draw);
  latest.current = draw;

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;

    let settled = false;

    const render = (elapsed: number): void => {
      const context = prepareCanvas(canvas);
      if (!context) return;
      const size = logicalSize(canvas);
      latest.current(context, size.width, size.height, elapsed);
      if (elapsed >= duration) settled = true;
    };

    onFrame(canvas, render);
    onVisibility(
      canvas,
      () => {
        if (!settled) resume(canvas);
      },
      () => pause(canvas),
    );
    render(0);

    return () => pause(canvas);
  }, [duration]);

  return ref;
}

/** Counts a number up once, when it first scrolls into view. */
export function useCountUp(target: number, duration = 900): React.RefObject<HTMLSpanElement | null> {
  const ref = useRef<HTMLSpanElement | null>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element || target === 0) return;

    // Respect the system setting rather than animating regardless: a counter that counts
    // up is exactly the kind of motion that setting exists to stop.
    if (globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches) {
      element.textContent = String(target);
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          observer.unobserve(element);
          const start = performance.now();
          const tick = (now: number): void => {
            const progress = Math.min(1, (now - start) / duration);
            const eased = 1 - (1 - progress) ** 3;
            element.textContent = String(Math.round(target * eased));
            if (progress < 1) requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        }
      },
      { threshold: 0.4 },
    );

    observer.observe(element);
    return () => observer.disconnect();
  }, [target, duration]);

  return ref;
}
