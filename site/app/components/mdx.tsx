/**
 * The components MDX pages are written in.
 *
 * A page of this site is a numbered walkthrough, so the vocabulary is small: `Steps`,
 * `Step`, `Figure`, `Note` and `Gotcha`. Writing those once here is what keeps eleven
 * pages from disagreeing about how a step is numbered or how a measurement is set — the
 * same argument as the renderer this replaced, except the enforcement is now the compiler
 * rather than discipline.
 *
 * The `number`/`setNumber` pair is React's documented pattern for auto-numbering children
 * that are not direct siblings. Without it the counter resets at every intervening element.
 */

import {
  Children,
  cloneElement,
  isValidElement,
  type ReactElement,
  type ReactNode,
} from "react";

/* --------------------------------------------------------------- step numbers */

/**
 * Auto-numbers the `Step` children.
 *
 * The obvious implementation — a counter in context, incremented as each step renders —
 * mutates state during render, which is the one thing React does not tolerate: the
 * increments land after the numbers have already been read, so every step prints `01`.
 *
 * Instead the numbering is done structurally: walk the children, find the steps, and hand
 * each one its index as an ordinary prop. It happens before render, needs no state, and
 * cannot be observed half-applied.
 *
 * Steps nested inside a wrapper are counted too, because the walk recurses rather than
 * looking only at direct children.
 */
function numberSteps(children: ReactNode, counter = { n: 0 }): ReactNode {
  return Children.map(children, (child) => {
    if (!isValidElement(child)) return child;

    // The props type is erased deliberately: this walk has to clone whatever component
    // an author put in the tree, and its props are not knowable from here.
    const element = child as ReactElement<Record<string, unknown>>;
    const kids = element.props["children"] as ReactNode | undefined;

    if (element.type === Step) {
      counter.n += 1;
      return cloneElement(element, { n: counter.n });
    }

    if (kids !== undefined) {
      return cloneElement(element, { children: numberSteps(kids, counter) });
    }
    return child;
  });
}

export function Steps({ children }: { children?: ReactNode }) {
  return <ol className="steps">{numberSteps(children)}</ol>;
}

/* -------------------------------------------------------------------- pieces */

export function Step({
  n,
  title,
  figure,
  children,
}: {
  /** Injected by `Steps`. Present so the number cannot drift from the order. */
  n?: number;
  title: string;
  /** A measured value shown beside the step, the way a datasheet puts a spec in the margin. */
  figure?: { value: string; unit?: string; caption: string };
  children?: ReactNode;
}) {
  const number = n ?? 0;
  return (
    <li className="step" id={`step-${number}`}>
      <div className="step__head">
        <span className="step__number">{String(number).padStart(2, "0")}</span>
        <h3 className="step__title">{title}</h3>
      </div>
      <div className="step__detail">{children}</div>
      {figure ? (
        <div className="figure">
          <span className="figure__value">{figure.value}</span>
          {figure.unit ? <span className="figure__unit">{figure.unit}</span> : null}
          <span className="figure__caption">{figure.caption}</span>
        </div>
      ) : null}
    </li>
  );
}

export function Figure({
  value,
  unit,
  caption,
}: {
  value: string;
  unit?: string;
  caption: string;
}) {
  return (
    <div className="figure">
      <span className="figure__value">{value}</span>
      {unit ? <span className="figure__unit">{unit}</span> : null}
      <span className="figure__caption">{caption}</span>
    </div>
  );
}

const NOTE_LABELS: Record<string, string> = {
  decision: "decision",
  measured: "measured",
  trap: "trap",
  gap: "gap",
  upstream: "upstream",
};

export function Note({
  kind = "measured",
  title,
  children,
}: {
  kind?: "decision" | "measured" | "trap" | "gap" | "upstream";
  title: string;
  children?: ReactNode;
}) {
  return (
    <li className={`note note--${kind}`}>
      <div className="note__head">
        <span className={`note__kind note__kind--${kind}`}>{NOTE_LABELS[kind] ?? kind}</span>
        <h3 className="note__title">{title}</h3>
      </div>
      <div className="note__body">{children}</div>
    </li>
  );
}

export function Gotcha({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <li className="gotcha">
      <h3 className="gotcha__title">{title}</h3>
      <div className="gotcha__body">{children}</div>
    </li>
  );
}

/** A short definition, for the terms a beginner has to have before the rest makes sense. */
export function Def({ term, children }: { term: string; children?: ReactNode }) {
  return (
    <div className="def">
      <dt className="def__term">{term}</dt>
      <dd className="def__body">{children}</dd>
    </div>
  );
}

/** A comparison table. Used where the honest answer is "both, and here is the trade". */
export function Compare({
  head,
  rows,
}: {
  head: [string, string, string];
  rows: [string, string, string][];
}) {
  return (
    <div className="table-wrap">
      <table className="compare">
        <thead>
          <tr>
            <th>{head[0]}</th>
            <th>{head[1]}</th>
            <th>{head[2]}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row[0]}>
              <th scope="row">{row[0]}</th>
              <td>{row[1]}</td>
              <td>{row[2]}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** The map the MDX runtime is given, so prose elements are styled without a wrapper. */
export const mdxComponents = {
  Steps,
  Step,
  Figure,
  Note,
  Gotcha,
  Def,
  Compare,
  h1: (props: React.ComponentProps<"h1">) => <h1 className="doc-header__title" {...props} />,
  h2: (props: React.ComponentProps<"h2">) => <h2 className="doc__h2" {...props} />,
  h3: (props: React.ComponentProps<"h3">) => <h3 className="doc__h3" {...props} />,
  pre: (props: React.ComponentProps<"pre">) => <pre className="mdx-pre" {...props} />,
  table: (props: React.ComponentProps<"table">) => <table className="compare" {...props} />,
} as const;
