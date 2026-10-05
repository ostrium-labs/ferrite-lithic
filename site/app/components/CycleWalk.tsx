/**
 * "Read one cycle" — text that drives a diagram.
 *
 * Four short paragraphs, and a timing diagram that advances as each one becomes the
 * paragraph you are reading. The text is not an illustration of the animation; it *is* the
 * control for it. That is the whole reason to build it this way: the reader sets the pace,
 * which is the one thing an animation that plays on its own never lets you do.
 *
 * What it teaches, in the order the steps state it:
 *   1. a cycle begins at a rising edge, and nothing else in the design changes state
 *   2. the clock period is the whole budget — combinational logic must settle inside it
 *   3. `d` is stable before the edge; that stability is what setup time is about
 *   4. at the edge, `q` takes the value `d` had, so the register is always one cycle behind
 *
 * Reduced motion: the diagram renders the active step without tweening, so the section is
 * still fully usable with animation off.
 */

import { useEffect, useRef, useState } from "react";

import { logicalSize, onFrame, onVisibility, palette, pause, prepareCanvas, resume } from "../lib/ticker";

const STEPS = [
  {
    label: "the edge",
    text: "A cycle starts at a rising clock edge. Inside the cycle, nothing changes state: every register holds what it captured at the last edge, and the only thing moving is combinational logic settling.",
  },
  {
    label: "the budget",
    text: "The clock period is the entire budget. Everything between this edge and the next has to settle inside it — which is why a design with a long combinational path is a design that will not meet timing, and why `pipeline` exists.",
  },
  {
    label: "before the edge",
    text: "`d` has to be stable for a while before the edge, not just correct at it. That lead time is setup time, and the bracket in the hero diagram is measuring it. Get it wrong and the register captures whatever was in flight.",
  },
  {
    label: "after the edge",
    text: "At the edge, every register simultaneously takes the value its input had. So `q` becomes the previous `d`. A register is not a wire that is slow — it is a value from one cycle ago, which is why the diagram always trails the input by one.",
  },
] as const;

const TOTAL = 8;

/** Draws the diagram with one cycle active. Pure function of `active`, so it is testable. */
function draw(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  active: number,
  /** 0 while the step is being arrived at, 1 once settled — drives the highlight easing. */
  settle: number,
): void {
  context.fillStyle = palette.bg;
  context.fillRect(0, 0, width, height);

  const pad = { top: 54, right: 26, bottom: 54, left: 74 };
  const plotWidth = width - pad.left - pad.right;
  if (plotWidth < 100) return;

  const cycleWidth = plotWidth / TOTAL;
  const plotHeight = height - pad.top - pad.bottom;

  const traces = [
    { name: "clk", y: 0.1, h: 0.15, kind: "clock" as const },
    { name: "d", y: 0.42, h: 0.16, kind: "bus" as const },
    { name: "q", y: 0.72, h: 0.16, kind: "bus" as const },
  ];
  const dValues = ["0xa3", "0xa3", "0x1f", "0x1f", "0xc0", "0xc0", "0x5a", "0x5a"];
  const qValues = ["—", "—", "0xa3", "0xa3", "0x1f", "0x1f", "0xc0", "0xc0"];

  /* ---- cycle boundaries; the active one is the trace colour. */
  context.font = `11px "IBM Plex Mono", monospace`;
  context.textBaseline = "alphabetic";

  for (let cycle = 0; cycle <= TOTAL; cycle += 1) {
    const x = pad.left + cycle * cycleWidth;
    const isActive = cycle === active;
    context.strokeStyle = isActive ? palette.accent : palette.lineSoft;
    context.lineWidth = isActive ? 1.5 : 1;
    context.beginPath();
    context.moveTo(Math.round(x) + 0.5, pad.top - 16);
    context.lineTo(Math.round(x) + 0.5, pad.top + plotHeight + 12);
    context.stroke();

    context.fillStyle = isActive ? palette.accent : palette.inkFaint;
    context.textAlign = "center";
    context.fillText(String(cycle), x, pad.top + plotHeight + 26);
  }

  /* ---- trace names */
  context.fillStyle = palette.inkFaint;
  context.textAlign = "right";
  for (const trace of traces) {
    context.fillText(trace.name, pad.left - 12, pad.top + trace.y * plotHeight + trace.h * plotHeight + 4);
  }

  /* ---- the traces */
  const from = pad.left;
  const to = pad.left + (active + 1) * cycleWidth;

  for (const trace of traces) {
    const y = pad.top + trace.y * plotHeight;
    const h = trace.h * plotHeight;

    if (trace.kind === "clock") {
      drawClock(context, from, to, y, h, cycleWidth);
    } else {
      const values = trace.name === "d" ? dValues : qValues;
      drawBus(context, values, from, to, y, h, cycleWidth, active);
    }
  }

  /* ---- the edge marker: the thing the whole section is about. */
  const edgeX = pad.left + active * cycleWidth;
  context.save();
  context.strokeStyle = palette.accent;
  context.lineWidth = 1.5;
  context.setLineDash([3, 3]);
  context.beginPath();
  context.moveTo(Math.round(edgeX) + 0.5, pad.top - 24);
  context.lineTo(Math.round(edgeX) + 0.5, pad.top + plotHeight + 14);
  context.stroke();
  context.restore();

  context.fillStyle = palette.accent;
  context.textAlign = edgeX > pad.left + plotWidth * 0.7 ? "right" : "left";
  context.fillText("rising edge", edgeX + (edgeX > pad.left + plotWidth * 0.7 ? -8 : 8), pad.top - 30);

  /* ---- the setup bracket, which is the subject of step 3.
     Fades in with `settle` so arriving at that step reads as the bracket being named. */
  if (active >= 1) {
    const alpha = active === 2 ? settle : 0.35;
    const by = pad.top + plotHeight + 42;
    const bx = pad.left + (active - 1) * cycleWidth;
    context.save();
    context.globalAlpha = alpha;
    context.strokeStyle = palette.inkDim;
    context.fillStyle = palette.inkDim;
    context.lineWidth = 1;
    context.beginPath();
    context.moveTo(bx, by);
    context.lineTo(edgeX, by);
    context.stroke();
    for (const x of [bx, edgeX]) {
      context.beginPath();
      context.moveTo(Math.round(x) + 0.5, by - 5);
      context.lineTo(Math.round(x) + 0.5, by + 5);
      context.stroke();
    }
    context.textAlign = "center";
    context.fillText("t_setup", (bx + edgeX) / 2, by - 8);
    context.restore();
  }
}

function drawClock(
  context: CanvasRenderingContext2D,
  from: number,
  to: number,
  y: number,
  h: number,
  cycleWidth: number,
): void {
  const low = y + h;
  const high = y;
  context.strokeStyle = palette.accent;
  context.lineWidth = 1.6;
  context.beginPath();
  context.moveTo(from, low);

  const half = cycleWidth / 2;
  let x = from;
  let level = low;
  while (x < to) {
    const nextX = Math.min(x + half, to);
    const nextLevel = level === low ? high : low;
    if (x <= to) context.lineTo(x, nextLevel);
    context.lineTo(nextX, nextLevel);
    x += half;
    level = nextLevel;
  }
  context.stroke();
}

function drawBus(
  context: CanvasRenderingContext2D,
  values: string[],
  from: number,
  to: number,
  y: number,
  h: number,
  cycleWidth: number,
  active: number,
): void {
  context.font = `12px "IBM Plex Mono", monospace`;
  context.textBaseline = "middle";
  context.textAlign = "center";

  for (let cycle = 0; cycle < values.length; cycle += 1) {
    const cellX = from + cycle * cycleWidth;
    if (cellX >= to) break;

    const cellWidth = Math.min(cycleWidth, to - cellX);
    const value = values[cycle];
    const isHold = cycle > 0 && values[cycle - 1] === value;
    // The cell the text is currently talking about is the lit one.
    const isActive = cycle === active;

    context.fillStyle = isActive ? palette.panel : palette.bg;
    context.fillRect(cellX, y, cellWidth - 1, h);

    context.strokeStyle = isActive ? palette.accent : isHold ? palette.line : palette.accent2;
    context.lineWidth = isActive ? 1.6 : 1;
    context.strokeRect(
      Math.round(cellX) + 0.5,
      Math.round(y) + 0.5,
      Math.round(cellWidth - 1),
      Math.round(h),
    );

    if (cellWidth > 24) {
      context.fillStyle = isActive ? palette.ink : isHold ? palette.inkFaint : palette.inkDim;
      context.fillText(value, cellX + (cellWidth - 1) / 2, y + h / 2 + 0.5);
    }
  }
}

export function CycleWalk() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [active, setActive] = useState(0);
  const stepRefs = useRef<(HTMLLIElement | null)[]>([]);

  /* ---- the text drives the diagram: whichever step is in the middle of the
     viewport is the active one. Chosen by offset rather than by intersection
     alone, because "most visible" is the question and "is intersecting" is not. */
  useEffect(() => {
    const nodes = stepRefs.current.filter((node): node is HTMLLIElement => node !== null);
    if (nodes.length === 0) return;

    const observer = new IntersectionObserver(
      (entries) => {
        let best: { index: number; ratio: number } | null = null;
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const index = nodes.indexOf(entry.target as HTMLLIElement);
          if (index < 0) continue;
          if (!best || entry.intersectionRatio > best.ratio) best = { index, ratio: entry.intersectionRatio };
        }
        if (best) setActive(best.index);
      },
      // A band across the middle of the viewport: "the step I am reading", not
      // "a step I have scrolled past".
      { rootMargin: "-45% 0px -45% 0px", threshold: [0, 0.25, 0.5, 1] },
    );

    for (const node of nodes) observer.observe(node);
    return () => observer.disconnect();
  }, []);

  /* ---- the canvas follows the text, with a short ease so a step change reads as
     movement rather than a jump. */
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const reduced = globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    let shown = active;
    let since = performance.now();

    const render = (elapsed: number): void => {
      const context = prepareCanvas(canvas);
      if (!context) return;
      const size = logicalSize(canvas);

      if (!reduced) {
        // 140ms is long enough to read as intent and short enough not to lag the text.
        const progress = Math.min(1, (elapsed - since) / 140);
        const eased = 1 - (1 - progress) ** 3;
        shown += (active - shown) * eased;
        if (Math.abs(active - shown) < 0.002) shown = active;
      } else {
        shown = active;
      }

      const settle = Math.min(1, (elapsed - since) / 240);
      draw(context, size.width, size.height, Math.round(shown), settle);
    };

    since = 0;
    onFrame(canvas, render);
    onVisibility(
      canvas,
      () => resume(canvas),
      () => pause(canvas),
    );

    const onResize = (): void => render(0);
    globalThis.addEventListener("resize", onResize);
    render(0);

    return () => {
      pause(canvas);
      globalThis.removeEventListener("resize", onResize);
    };
  }, [active]);

  return (
    <section className="section section--walk" id="cycle">
      <div className="walk">
        <div className="walk__text">
          <h2>Read one cycle</h2>
          <p className="prose">
            Four things are true about a clock edge, and they are easier to see than to
            read. Each paragraph below moves the diagram.
          </p>
          <ol className="walk__steps">
            {STEPS.map((step, index) => (
              <li
                key={step.label}
                ref={(node) => {
                  stepRefs.current[index] = node;
                }}
                className={`walk__step${index === active ? " is-active" : ""}`}
              >
                <span className="walk__label">{step.label}</span>
                <p>{step.text}</p>
              </li>
            ))}
          </ol>
        </div>

        <figure className="walk__figure">
          <canvas ref={canvasRef} width={900} height={420} aria-hidden="true" />
          <figcaption>
            Cycle {active} of 8. <code>d</code> is the input bus and <code>q</code> is the
            register, so <code>q</code> always shows what <code>d</code> held one cycle ago.
          </figcaption>
        </figure>
      </div>
    </section>
  );
}
