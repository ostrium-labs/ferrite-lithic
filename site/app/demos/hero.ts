/**
 * The hero timing diagram.
 *
 * This is the one place on the site that is allowed to be loud, and it is loud
 * because of what it depicts rather than because of how it moves. The most
 * characteristic thing in the world of this project is a clock edge: everything
 * the simulator, the emitter and the equivalence check agree about is *when* a
 * value changes. So the hero is a timing diagram — clock, data, and the
 * measurement brackets a datasheet uses to say `t_setup` — and it draws itself
 * once, left to right, the way a plotter or a scope's persistence would.
 *
 * Why this and not a logo, a screenshot, or a big number:
 *  - A screenshot of Verilog would advertise a language this project is not.
 *  - A big number with a label is the treatment every documentation site reaches
 *    for first, which is exactly what makes it read as a template.
 *  - A timing diagram is the artefact a hardware engineer actually reads, and it
 *    states the project's thesis — one graph, two backends, one clock — without
 *    a single word of copy.
 *
 * It animates exactly once. After that it holds, because a hero that loops is a
 * hero competing with the paragraph under it.
 */

import { logicalSize, onFrame, onVisibility, palette, pause, prepareCanvas, resume } from "../lib/ticker";

const MONO = '"IBM Plex Mono", ui-monospace, Menlo, monospace';

/** How long the drawing takes, in ms. Long enough to read as deliberate. */
const DURATION = 2200;

const PAD = { top: 46, right: 30, bottom: 62, left: 92 };

interface Trace {
  name: string;
  y: number;
  height: number;
  /** Bus traces show a value per cycle; clock traces show edges. */
  kind: "clock" | "bus" | "wire";
  values: string[];
  /** Where the sequential boundary sits, as a fraction of the trace width. */
  mark?: number;
}

export function drawTimingHero(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  elapsed: number,
): void {
  context.clearRect(0, 0, width, height);
  context.fillStyle = palette.bg;
  context.fillRect(0, 0, width, height);

  const plotWidth = width - PAD.left - PAD.right;
  if (plotWidth < 120) return;

  const CYCLES = 8;
  const cycleWidth = plotWidth / CYCLES;

  /** 0 at the start of the draw, 1 when it is complete. */
  const progress = Math.min(1, elapsed / DURATION);
  /** Eased so the pen starts and stops rather than starting and stopping abruptly. */
  const eased = progress < 1 ? 1 - (1 - progress) ** 3 : 1;
  const drawn = eased * plotWidth;

  const traces: Trace[] = [
    { name: "clk", y: 0.16, height: 0.16, kind: "clock", values: [] },
    { name: "rst", y: 0.38, height: 0.1, kind: "bus", values: ["0", "0", "1", "1", "0", "0", "0", "0"] },
    { name: "d", y: 0.55, height: 0.12, kind: "bus", values: ["0xa3", "0x1f", "0x1f", "0xc0", "0x5a", "0x5a", "0x07", "0x91"] },
    { name: "q", y: 0.76, height: 0.12, kind: "bus", values: ["—", "—", "0xa3", "0x1f", "0x1f", "0xc0", "0x5a", "0x5a"] },
  ];

  const top = PAD.top;
  const plotHeight = height - PAD.top - PAD.bottom;

  /* ---- the time axis: the grid a timing diagram is read against. */
  context.strokeStyle = palette.lineSoft;
  context.lineWidth = 1;
  context.font = `11px ${MONO}`;
  context.textBaseline = "alphabetic";

  for (let cycle = 0; cycle <= CYCLES; cycle += 1) {
    const x = PAD.left + cycle * cycleWidth;
    if (x > PAD.left + drawn) break;
    context.beginPath();
    context.moveTo(Math.round(x) + 0.5, top - 14);
    context.lineTo(Math.round(x) + 0.5, top + plotHeight + 10);
    context.stroke();
  }

  /* ---- trace names, in the left margin, as a datasheet labels its pins. */
  context.fillStyle = palette.inkFaint;
  context.textAlign = "right";
  for (const trace of traces) {
    const y = top + trace.y * plotHeight;
    if (y > top + plotHeight) continue;
    context.fillText(trace.name, PAD.left - 14, y + 4);
  }

  /* ---- the traces themselves. */
  for (const trace of traces) {
    const y = top + trace.y * plotHeight;
    const h = trace.height * plotHeight;

    if (trace.kind === "clock") drawClock(context, y, h, cycleWidth, drawn);
    else drawBus(context, trace, y, h, cycleWidth, drawn);
  }

  /* ---- the sequential boundary.
    The whole project is the claim that the simulator and the emitted Verilog
    agree about this line, so it is the one annotation the diagram makes. */
  const edgeX = PAD.left + cycleWidth * 3;
  if (edgeX < PAD.left + drawn) {
    context.save();
    context.strokeStyle = palette.accent;
    context.lineWidth = 1.5;
    context.setLineDash([3, 3]);
    context.beginPath();
    context.moveTo(Math.round(edgeX) + 0.5, top - 20);
    context.lineTo(Math.round(edgeX) + 0.5, top + plotHeight + 16);
    context.stroke();
    context.restore();

    context.fillStyle = palette.accent;
    context.textAlign = "left";
    context.fillText("posedge: the only place state changes", edgeX + 7, top - 26);
  }

  /* ---- the measurement bracket: t_setup, drawn the way a datasheet draws it,
    with witness lines and arrows. This is the annotation that says "these two
    things are related", which is the entire subject of the equivalence check. */
  const bracketY = top + plotHeight + 34;
  const from = PAD.left + cycleWidth * 2;
  const to = edgeX;
  if (to < PAD.left + drawn) {
    context.strokeStyle = palette.inkDim;
    context.lineWidth = 1;

    for (const x of [from, to]) {
      context.beginPath();
      context.moveTo(Math.round(x) + 0.5, bracketY - 6);
      context.lineTo(Math.round(x) + 0.5, bracketY + 6);
      context.stroke();
    }

    context.beginPath();
    context.moveTo(from, Math.round(bracketY) + 0.5);
    context.lineTo(to, Math.round(bracketY) + 0.5);
    context.stroke();

    // Arrowheads, drawn as filled triangles so they read at any zoom.
    context.fillStyle = palette.inkDim;
    for (const [x, direction] of [
      [from, 1],
      [to, -1],
    ] as const) {
      context.beginPath();
      context.moveTo(x, bracketY);
      context.lineTo(x + 5 * direction, bracketY - 3);
      context.lineTo(x + 5 * direction, bracketY + 3);
      context.closePath();
      context.fill();
    }

    context.fillStyle = palette.inkDim;
    context.textAlign = "center";
    context.fillText("t_setup", (from + to) / 2, bracketY - 9);
  }

  /* ---- axis labels, in the signal colour: the only other place the accent
    appears in the diagram, because these are the reader's coordinates. */
  context.fillStyle = palette.inkFaint;
  context.textAlign = "center";
  for (let cycle = 0; cycle <= CYCLES; cycle += 1) {
    const x = PAD.left + cycle * cycleWidth;
    if (x > PAD.left + drawn) break;
    context.fillText(String(cycle), x, top + plotHeight + 22);
  }
  context.textAlign = "right";
  context.fillText("cycles", PAD.left - 14, top + plotHeight + 22);
}

/** A clock: square wave, one rising edge per cycle, exactly as a scope draws it. */
function drawClock(
  context: CanvasRenderingContext2D,
  y: number,
  height: number,
  cycleWidth: number,
  drawn: number,
): void {
  const low = y + height;
  const high = y;
  context.strokeStyle = palette.accent;
  context.lineWidth = 1.6;

  const startX = PAD.left;
  const limit = startX + drawn;
  const half = cycleWidth / 2;

  // A square wave, walked one half-cycle at a time: a vertical edge, then a
  // horizontal run. Stepping by half-period rather than by whole cycle is what
  // keeps the two edges the same distance apart, which is the entire definition
  // of a clock a reader checks by eye.
  context.beginPath();
  context.moveTo(startX, low);

  let x = startX;
  let level = low;
  while (x < limit) {
    const nextX = Math.min(x + half, limit);
    const nextLevel = level === low ? high : low;
    // The vertical edge is only drawn once the pen has actually reached it.
    if (x <= limit) context.lineTo(x, nextLevel);
    context.lineTo(nextX, nextLevel);
    x += half;
    level = nextLevel;
  }
  context.stroke();
}

/** A bus: one cell per cycle, value set in the middle, transitions drawn. */
function drawBus(
  context: CanvasRenderingContext2D,
  trace: Trace,
  y: number,
  height: number,
  cycleWidth: number,
  drawn: number,
): void {
  const startX = PAD.left;
  const limit = startX + drawn;

  context.font = `12px ${MONO}`;
  context.textBaseline = "middle";
  context.textAlign = "center";

  for (let cycle = 0; cycle < trace.values.length; cycle += 1) {
    const cellX = startX + cycle * cycleWidth;
    if (cellX > limit) break;

    const cellWidth = Math.min(cycleWidth, limit - cellX);
    const value = trace.values[cycle];
    const isHold = cycle > 0 && trace.values[cycle - 1] === value;

    // A held value is drawn hollow; a changed value filled. That single
    // distinction is what makes a bus diagram readable at a glance, and it is
    // why `q` visibly lags `d` by one cycle.
    context.fillStyle = isHold ? palette.bg : palette.panel;
    context.fillRect(cellX, y, cellWidth - 1, height);

    context.strokeStyle = isHold ? palette.line : palette.accent;
    context.lineWidth = isHold ? 1 : 1.4;
    context.strokeRect(
      Math.round(cellX) + 0.5,
      Math.round(y) + 0.5,
      Math.round(cellWidth - 1),
      Math.round(height),
    );

    // The value only appears once its cell is wide enough to hold it, so text
    // never spills out of a cell that is still being drawn.
    if (cellWidth > 26) {
      context.fillStyle = isHold ? palette.inkFaint : palette.ink;
      context.fillText(value, cellX + (cellWidth - 1) / 2, y + height / 2 + 0.5);
    }
  }
}

/**
 * Mounts the hero. It runs once and then holds — `paused` stops the shared
 * animation loop from ever being asked to draw it again.
 */
export function mountTimingHero(canvas: HTMLCanvasElement): void {
  if (!canvas) return;
  let settled = false;

  // Drawn in the canvas's own logical space, exactly like every other diagram, so the
  // hero does not need its own scaling story.
  const render = (elapsed: number): void => {
    const context = prepareCanvas(canvas);
    if (!context) return;
    const size = logicalSize(canvas);
    drawTimingHero(context, size.width, size.height, elapsed);
    if (elapsed >= DURATION) settled = true;
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
}

