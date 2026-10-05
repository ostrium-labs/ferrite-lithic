/**
 * The pipeline diagram, animated on a clock.
 *
 * The honest shape of the claim: one graph in the middle, two consumers, and an
 * equivalence check that compares them cycle by cycle. Data pulses run left to right
 * and the two branches are compared at the end, so the page shows the *structure* of
 * the verification rather than asserting that it happens.
 */

import { MONO, palette, roundRect } from "../lib/ticker";

interface Node {
  x: number;
  y: number;
  w: number;
  h: number;
  title: string;
  sub: string;
  tint: string;
}

const NODES: Node[] = [
  { x: 20, y: 96, w: 150, h: 74, title: "Rust builder", sub: "Design::sll(…)", tint: palette.accent3 },
  { x: 214, y: 96, w: 168, h: 74, title: "Design", sub: "nodes + widths", tint: palette.accent },
  {
    x: 426,
    y: 26,
    w: 176,
    h: 74,
    title: "Sim",
    sub: "cycle simulator",
    tint: palette.accent2,
  },
  {
    x: 426,
    y: 166,
    w: 176,
    h: 74,
    title: "RTL emitter",
    sub: "structural Verilog",
    tint: palette.accent2,
  },
  { x: 646, y: 26, w: 152, h: 74, title: "Sim runs", sub: "cycle by cycle", tint: palette.accent2 },
  { x: 646, y: 166, w: 152, h: 74, title: "Verilator", sub: "compiles + runs", tint: palette.accent2 },
  {
    x: 842,
    y: 96,
    w: 178,
    h: 74,
    title: "Plan + Stimulus",
    sub: "same columns",
    tint: palette.warn,
  },
  {
    x: 1064,
    y: 96,
    w: 118,
    h: 74,
    title: "equivalent?",
    sub: "or first diff",
    tint: palette.good,
  },
];

const EDGES: [number, number][] = [
  [0, 1],
  [1, 2],
  [1, 3],
  [2, 4],
  [3, 5],
  [4, 6],
  [5, 6],
  [6, 7],
];

/** A cubic curve from the right edge of one node to the left edge of another. */
function edgePath(from: Node, to: Node): string {
  const x0 = from.x + from.w;
  const y0 = from.y + from.h / 2;
  const x1 = to.x;
  const y1 = to.y + to.h / 2;
  const bend = Math.max(30, (x1 - x0) * 0.55);
  return `M ${x0} ${y0} C ${x0 + bend} ${y0}, ${x1 - bend} ${y1}, ${x1} ${y1}`;
}

export function drawPipeline(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  elapsed: number,
): void {
  context.fillStyle = palette.bg;
  context.fillRect(0, 0, width, height);

  const scale = width / 1200;
  context.save();
  context.scale(scale, scale);
  const w = 1200;
  const h = 420;

  // The pulse travels left to right; each edge gets its own offset so the whole graph
  // reads as one signal rather than eight independent ones.
  const cycle = (elapsed / 26) % (EDGES.length * 3 + 10);

  // ---- edges
  for (const [index, [fromIndex, toIndex]] of EDGES.entries()) {
    const from = NODES[fromIndex];
    const to = NODES[toIndex];
    const path = edgePath(from, to);

    context.strokeStyle = palette.line;
    context.lineWidth = 2;
    context.beginPath();
    context.moveTo(from.x + from.w, from.y + from.h / 2);
    context.bezierCurveTo(
      from.x + from.w + Math.max(30, (to.x - from.x - from.w) * 0.55),
      from.y + from.h / 2,
      to.x - Math.max(30, (to.x - from.x - from.w) * 0.55),
      to.y + to.h / 2,
      to.x,
      to.y + to.h / 2,
    );
    context.stroke();

    // The travelling pulse, clipped to the path so it cannot overshoot the node.
    const local = ((cycle - index * 0.7) % (EDGES.length * 3 + 10)) / (EDGES.length * 3 + 10);
    if (local >= 0 && local <= 1) {
      const point = pointOnCubic(
        from.x + from.w,
        from.y + from.h / 2,
        from.x + from.w + Math.max(30, (to.x - from.x - from.w) * 0.55),
        from.y + from.h / 2,
        to.x - Math.max(30, (to.x - from.x - from.w) * 0.55),
        to.y + to.h / 2,
        to.x,
        to.y + to.h / 2,
        local,
      );
      const glow = 1 - Math.abs(local - 0.5) * 0.7;
      context.fillStyle = palette.accent;
      context.globalAlpha = 0.25 + glow * 0.55;
      context.beginPath();
      context.arc(point.x, point.y, 4.5, 0, Math.PI * 2);
      context.fill();
      context.globalAlpha = 1;
    }
    void path;
  }

  // ---- nodes
  for (const [index, node] of NODES.entries()) {
    // A slow breathing highlight on the graph's source, so there is a visual clock.
    const phase = Math.sin((elapsed / 700) + index * 0.4) * 0.5 + 0.5;

    context.fillStyle = palette.panel;
    roundRect(context, node.x, node.y, node.w, node.h, 10);
    context.fill();

    context.strokeStyle = node.tint;
    context.globalAlpha = 0.35 + phase * 0.5;
    context.lineWidth = 1.6;
    roundRect(context, node.x, node.y, node.w, node.h, 10);
    context.stroke();
    context.globalAlpha = 1;

    // The tint as a bar, which is what makes the two branches readable as a pair.
    context.fillStyle = node.tint;
    roundRect(context, node.x, node.y, 4, node.h, 2);
    context.fill();

    context.font = `600 14px ${MONO}`;
    context.fillStyle = palette.ink;
    context.textAlign = "left";
    context.textBaseline = "middle";
    context.fillText(node.title, node.x + 16, node.y + node.h / 2 - 9);

    context.font = `11px ${MONO}`;
    context.fillStyle = palette.inkFaint;
    context.fillText(node.sub, node.x + 16, node.y + node.h / 2 + 11);
  }

  // ---- the comparison, checked at the right edge
  const checkPhase = ((elapsed / 1400) % 1);
  const check = NODES[7];
  const sweeping = checkPhase < 0.5;
  context.strokeStyle = sweeping ? palette.good : palette.line;
  context.lineWidth = sweeping ? 2 : 1.2;
  roundRect(context, check.x, check.y, check.w, check.h, 10);
  context.stroke();

  // A scan line across the comparison box, so "checked every cycle" is visible.
  if (sweeping) {
    const scanX = check.x + ((checkPhase / 0.5) * check.w);
    context.strokeStyle = palette.good;
    context.globalAlpha = 0.6;
    context.lineWidth = 1.5;
    context.beginPath();
    context.moveTo(scanX, check.y - 6);
    context.lineTo(scanX, check.y + check.h + 6);
    context.stroke();
    context.globalAlpha = 1;
  }

  // ---- the legend: what the two branches are for
  context.font = `11px ${MONO}`;
  context.fillStyle = palette.inkFaint;
  context.textAlign = "left";
  context.fillText(
    "the simulator is what the tests believe; Verilator is what a synthesiser would run",
    20,
    h - 22,
  );
  context.textAlign = "right";
  context.fillStyle = palette.inkFaint;
  context.fillText("one graph, two consumers, one arbiter", w - 20, h - 22);

  context.restore();
  void height;
}

/** Point on a cubic Bezier, for the travelling pulse. */
function pointOnCubic(
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  x3: number,
  y3: number,
  t: number,
): { x: number; y: number } {
  const u = 1 - t;
  const a = u * u * u;
  const b = 3 * u * u * t;
  const c = 3 * u * t * t;
  const d = t * t * t;
  return {
    x: a * x0 + b * x1 + c * x2 + d * x3,
    y: a * y0 + b * y1 + c * y2 + d * y3,
  };
}
