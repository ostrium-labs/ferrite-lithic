/**
 * One shared requestAnimationFrame loop.
 *
 * Every canvas on the page would otherwise run its own loop, and six of them running
 * at once on a laptop is six times the work for the same picture. This multiplexes
 * them onto one frame callback, and — the part that actually matters — stops
 * animating anything that is off screen.
 */

type Frame = (elapsedMs: number, deltaMs: number) => void;

interface Entry {
  draw: Frame;
  running: boolean;
  last: number;
}

const entries = new Map<object, Entry>();
let rafId = 0;
let startedAt = 0;

function pump(now: number): void {
  if (startedAt === 0) startedAt = now;

  for (const entry of entries.values()) {
    if (!entry.running) continue;
    const delta = now - entry.last;
    // A tab that was backgrounded hands back a huge delta; clamping keeps a demo from
    // jumping hundreds of steps at once when it is looked at again.
    entry.draw(now - startedAt, Math.min(delta, 64));
    entry.last = now;
  }

  rafId = entries.size > 0 ? requestAnimationFrame(pump) : 0;
}

function ensureRunning(): void {
  if (rafId === 0 && entries.size > 0) {
    startedAt = 0;
    rafId = requestAnimationFrame(pump);
  }
}

/** Registers a draw callback for `owner`, keyed so it can be replaced or removed. */
export function onFrame(owner: object, draw: Frame): void {
  entries.set(owner, { draw, running: false, last: 0 });
  ensureRunning();
}

/** Stops drawing for `owner` without forgetting it. */
export function pause(owner: object): void {
  const entry = entries.get(owner);
  if (entry) entry.running = false;
}

/** Resumes drawing for `owner`. */
export function resume(owner: object): void {
  const entry = entries.get(owner);
  if (entry) {
    entry.running = true;
    entry.last = 0;
  }
}

/** Removes `owner` from the loop entirely. */
export function offFrame(owner: object): void {
  entries.delete(owner);
  if (entries.size === 0 && rafId !== 0) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
}

/**
 * Calls `onEnter` the first time `target` scrolls into view and again every time it
 * leaves, so a demo can pause its own work while nobody is looking at it.
 */
export function onVisibility(target: Element, onEnter: () => void, onLeave: () => void): void {
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) onEnter();
        else onLeave();
      }
    },
    { threshold: 0.05 },
  );
  observer.observe(target);
}

/**
 * Sizes a canvas's backing store to its laid-out size and returns a context whose
 * transform maps the canvas's **attribute** coordinate space onto the display.
 *
 * The distinction matters. CSS stretches a canvas to fill its box, so a 1100x300 canvas
 * displayed at 960px wide is really 960x262 — and a demo that mixes absolute layout
 * constants with the rendered height puts its footer through its own contents. Fixing
 * the transform here means every demo lays out in the coordinate space its constants were
 * written in, whatever size the canvas ends up.
 */
export function prepareCanvas(canvas: HTMLCanvasElement): CanvasRenderingContext2D | null {
  const context = canvas.getContext("2d");
  if (!context) return null;

  if (canvas.dataset.logicalWidth === undefined) {
    canvas.dataset.logicalWidth = String(canvas.width);
    canvas.dataset.logicalHeight = String(canvas.height);
  }

  const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
  const displayWidth = canvas.clientWidth || Number(canvas.dataset.logicalWidth);
  const displayHeight = canvas.clientHeight || Number(canvas.dataset.logicalHeight);
  const backingWidth = Math.round(displayWidth * ratio);
  const backingHeight = Math.round(displayHeight * ratio);

  if (canvas.width !== backingWidth || canvas.height !== backingHeight) {
    canvas.width = backingWidth;
    canvas.height = backingHeight;
  }

  const scale = displayWidth / Number(canvas.dataset.logicalWidth);
  context.setTransform(ratio * scale, 0, 0, ratio * scale, 0, 0);
  context.clearRect(0, 0, Number(canvas.dataset.logicalWidth), Number(canvas.dataset.logicalHeight));
  return context;
}

/**
 * The coordinate space a demo should draw in. The backdrop overrides this to the window
 * size, since it is fixed to the viewport rather than to an attribute box.
 */
export function logicalSize(canvas: HTMLCanvasElement): { width: number; height: number } {
  if (canvas.dataset.logicalWidth === undefined) {
    canvas.dataset.logicalWidth = String(canvas.width);
    canvas.dataset.logicalHeight = String(canvas.height);
  }
  return {
    width: Number(canvas.dataset.logicalWidth),
    height: Number(canvas.dataset.logicalHeight),
  };
}

/**
 * The instrument palette, read once so the demos cannot drift apart visually.
 *
 * These are the `tokens.css` values under `.instrument` — the same six colours
 * with the ground flipped. The demos draw on a screen rather than on paper, so
 * they get the screen side of the one palette; the trace colour is amber in
 * both, which is what makes a figure here feel like it belongs to the page.
 *
 * There is no third accent. `accent2` is the same amber at a different job
 * (a wire rather than a signal) and `accent3` is the fault colour, so a figure
 * cannot introduce a hue the design system does not have.
 */
export const palette = {
  bg: "#14181a",
  panel: "#1b2022",
  line: "#333c3e",
  lineSoft: "#242c2e",
  ink: "#e9ebe9",
  inkDim: "#a4ada9",
  inkFaint: "#6d7674",
  accent: "#e8894a", // the trace
  accent2: "#c98a5e", // a wire: the trace, quieter
  accent3: "#d4695a", // the fault colour
  hot: "#d4695a",
  good: "#7fa88c",
  warn: "#e0b357",
} as const;

/** Rounded rectangle, because `roundRect` is not universal and this is not worth a shim file. */
export function roundRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  const r = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  context.moveTo(x + r, y);
  context.arcTo(x + width, y, x + width, y + height, r);
  context.arcTo(x + width, y + height, x, y + height, r);
  context.arcTo(x, y + height, x, y, r);
  context.arcTo(x, y, x + width, y, r);
  context.closePath();
}

/** Monospaced text, because every number on this page is a measurement. */
export const MONO = 'ui-monospace, "SF Mono", Menlo, Consolas, monospace';