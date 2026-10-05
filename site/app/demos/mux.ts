/**
 * The honest gap, drawn: `Design::rom` is a multiplexer tree.
 *
 * The point this makes is a *cost*, so it has to be a cost you can watch accumulate.
 * Each cell is one `case_` arm; the design is correct and its shape is right, but a
 * 256-entry table is 256 arms here and one block RAM in silicon. The animation grows
 * the table rather than asserting the number.
 */

const SIZES: { entries: number; label: string }[] = [
  { entries: 16, label: "a nibble LUT" },
  { entries: 64, label: "a sextet alphabet map" },
  { entries: 256, label: "AES's S-box" },
];

export function mountMux(container: HTMLElement): void {
  const fragment = document.createDocumentFragment();

  for (const size of SIZES) {
    const row = document.createElement("div");
    row.className = "muxrow";

    const label = document.createElement("span");
    label.className = "muxrow__label";
    label.textContent = `${size.label} — ${size.entries} entries`;
    row.append(label);

    const grid = document.createElement("div");
    grid.className = "muxgrid";
    row.append(grid);

    const note = document.createElement("p");
    note.className = "muxnote";
    note.textContent =
      size.entries <= 64
        ? "close enough to the truth to use without comment"
        : "a deep mux where real silicon infers block RAM";
    row.append(note);

    fragment.append(row);
  }

  container.replaceChildren(fragment);

  // Fill the grids progressively, and keep them filled: a table that empties itself is a
  // table you cannot count. The sizes are staggered so the eye reads small to large.
  const grids = Array.from(container.querySelectorAll<HTMLElement>(".muxgrid"));
  let offset = 0;

  for (const [rowIndex, grid] of grids.entries()) {
    const entries = SIZES[rowIndex].entries;
    const cells: HTMLElement[] = [];

    for (let index = 0; index < entries; index += 1) {
      const cell = document.createElement("span");
      cell.className = "muxcell";
      // One cell in sixteen is tinted, to suggest a table holding data rather than an
      // undifferentiated block of arms. Sparse on purpose: a fifth of the cells lit
      // reads as noise instead of as a value.
      if (index % 16 === 5) cell.classList.add("muxcell--hot");
      grid.append(cell);
      cells.push(cell);

      const revealAt = offset + index * 7;
      window.setTimeout(() => cell.classList.add("muxcell--on"), revealAt);
    }

    offset += entries * 7 + 420;
  }

  // The comparison, in the same visual language so the two costs are legible together.
  const note = document.createElement("p");
  note.className = "muxnote";
  note.style.marginTop = "0.5rem";
  note.innerHTML =
    "Against that: a 256×8 block RAM is <strong>one</strong> memory node with a write " +
    "port and a read port. The IR has no initialised-memory node — <code>Design::mem</code> " +
    "is zero-filled in both backends — so this is the single highest-value addition left " +
    "to the toolchain.";
  container.append(note);
}