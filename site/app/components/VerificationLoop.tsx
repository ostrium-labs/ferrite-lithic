import { Link } from "react-router";

/** Three different contracts. Equivalence alone cannot establish algorithm correctness. */
export function VerificationLoop() {
  return <div className="verification-loop">
    <p className="verification-loop__input">One corpus algorithm <span aria-hidden="true">↓</span></p>
    <ol>
      <li><span className="data-label">Function</span><h3>External reference</h3><p>Does the design compute the same result as an independent implementation?</p><Link to="/docs/corpus#step-8">Golden models →</Link></li>
      <li><span className="data-label">Timing</span><h3>Step testbench</h3><p>Does its handshake and state advance on the right clock edge?</p><Link to="/docs/tb">Cycle assertions →</Link></li>
      <li><span className="data-label">Backend agreement</span><h3>Simulator ⇄ Verilator</h3><p>Does the emitted RTL match the simulator under the same stimulus?</p><Link to="/docs/cosim">Equivalence checks →</Link></li>
    </ol>
    <p className="verification-loop__ceiling">Where no independent reference exists, the corpus states the limit of its evidence. Agreement between backends is not proof of the intended algorithm.</p>
  </div>;
}
