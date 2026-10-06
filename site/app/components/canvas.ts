import { useEffect, useRef } from "react";
import { logicalSize, offFrame, onFrame, onVisibility, pause, prepareCanvas, resume } from "../lib/ticker";

type Draw = (context: CanvasRenderingContext2D, width: number, height: number, elapsed: number) => void;

/** Owns every external subscription for an instrument. Finite traces settle and hold. */
function useInstrumentCanvas(draw: Draw, duration: number | undefined, replay: number, paused: boolean) {
  const ref = useRef<HTMLCanvasElement>(null);
  const latest = useRef(draw);
  latest.current = draw;
  const controls = useRef<(() => void) | null>(null);
  const repaint = useRef<(() => void) | null>(null);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  const elapsedRef = useRef(0);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    let visible = false;
    let disposed = false;
    let lastRender = -Infinity;
    elapsedRef.current = 0;
    const media = globalThis.matchMedia("(prefers-reduced-motion: reduce)");
    const render = (elapsed: number) => {
      elapsedRef.current = elapsed;
      const context = prepareCanvas(canvas);
      if (!context) return;
      const size = logicalSize(canvas);
      latest.current(context, size.width, size.height, elapsed);
    };
    const settled = () => duration !== undefined && elapsedRef.current >= duration;
    const sync = () => {
      if (visible && !pausedRef.current && !media.matches && !settled()) resume(canvas);
      else pause(canvas);
    };
    // A live demo's first instant can be an empty load/lookup phase. Reduced motion
    // starts at a useful completed lookup instead, then preserves that inspection.
    const renderStatic = () => render(media.matches ? duration ?? (elapsedRef.current || 750) : elapsedRef.current);
    onFrame(canvas, elapsed => {
      if (elapsed - lastRender >= 32 || (duration !== undefined && elapsed >= duration)) {
        render(duration === undefined ? elapsed : Math.min(duration, elapsed));
        lastRender = elapsed;
      }
      if (settled()) pause(canvas);
    });
    controls.current = sync;
    repaint.current = renderStatic;
    const disconnect = onVisibility(canvas, () => { visible = true; sync(); }, () => { visible = false; sync(); });
    const resize = new ResizeObserver(renderStatic);
    resize.observe(canvas);
    const motionChanged = () => { renderStatic(); sync(); };
    media.addEventListener("change", motionChanged);
    renderStatic();
    // Font loading affects canvas text but does not resize the surface.
    void document.fonts.ready.then(() => { if (!disposed) renderStatic(); });
    return () => {
      disposed = true;
      controls.current = null;
      repaint.current = null;
      disconnect();
      resize.disconnect();
      media.removeEventListener("change", motionChanged);
      offFrame(canvas);
    };
  }, [duration, replay]);
  useEffect(() => { controls.current?.(); }, [paused]);
  useEffect(() => { repaint.current?.(); }, [draw]);
  return ref;
}

export function useCanvas(draw: Draw, paused = false) {
  return useInstrumentCanvas(draw, undefined, 0, paused);
}

export function useOnceCanvas(duration: number, draw: Draw, replay = 0) {
  return useInstrumentCanvas(draw, duration, replay, false);
}
