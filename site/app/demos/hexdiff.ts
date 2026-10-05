/**
 * The FNV-1a offset-basis bug, animated.
 *
 * This is the sharpest defect in the project's history and it is worth showing rather
 * than describing: a published constant mistyped by one hex digit, in a test whose own
 * first assertion used the same wrong constant. The animation computes the hash both
 * ways, so the one-digit difference is visible in the answer rather than asserted.
 */

const OFFSET_WRONG = 0xcbf2_9ce4_8423_2325n;
const OFFSET_RIGHT = 0xcbf2_9ce4_8422_2325n;
const PRIME = 0x0000_0100_0000_01b3n;
const MASK = (1n << 64n) - 1n;

/** FNV-1a over one byte, with an arbitrary offset basis — which is the whole point. */
function fnv1a(bytes: number[], offsetBasis: bigint): bigint {
  let hash = offsetBasis & MASK;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * PRIME) & MASK;
  }
  return hash;
}

/** The published check value for FNV-1a 64 over the single byte `a`. */
const PUBLISHED = 0xaf63_dc4c_8601_ec8cn;

function hex16(value: bigint): string {
  return value.toString(16).padStart(16, "0");
}

/** Index of the first nibble where two 16-digit hex strings disagree, or -1. */
function differingNibble(left: string, right: string): number {
  for (let index = 0; index < 16; index += 1) {
    if (left[index] !== right[index]) return index;
  }
  return -1;
}

function nibbleRow(label: string, hex: string, note: string): HTMLElement {
  const row = document.createElement("div");
  row.className = "hexrow";

  const tag = document.createElement("span");
  tag.className = "hexrow__label";
  tag.textContent = label;
  row.append(tag);

  for (const character of hex) {
    const nibble = document.createElement("span");
    nibble.className = "hexnib";
    nibble.textContent = character;
    row.append(nibble);
  }

  const caption = document.createElement("span");
  caption.className = "hexrow__note";
  caption.textContent = note;
  row.append(caption);

  return row;
}

function hashRow(label: string, value: bigint, verdict: string, good: boolean): HTMLElement {
  const row = document.createElement("div");
  row.className = "hexrow";

  const tag = document.createElement("span");
  tag.className = "hexrow__label";
  tag.textContent = label;
  row.append(tag);

  const hex = hex16(value);
  for (const [index, character] of [...hex].entries()) {
    const nibble = document.createElement("span");
    // Highlight the nibbles that differ from the published value, which is more
    // informative than highlighting the one that was typed wrong: the input is off by
    // one digit, the output is off by several.
    const publishedCharacter = PUBLISHED.toString(16).padStart(16, "0")[index];
    nibble.className =
      character === publishedCharacter ? "hexnib" : good ? "hexnib hexnib--fixed" : "hexnib hexnib--diff";
    nibble.textContent = character;
    row.append(nibble);
  }

  const caption = document.createElement("span");
  caption.className = "hexrow__note";
  caption.textContent = verdict;
  row.append(caption);

  return row;
}

export function mountHexDiff(container: HTMLElement): void {
  const wrongHex = hex16(OFFSET_WRONG);
  const rightHex = hex16(OFFSET_RIGHT);
  const digit = differingNibble(wrongHex, rightHex);

  const rows: HTMLElement[] = [
    nibbleRow("as typed", wrongHex, `digit ${digit + 1} is 3, should be 2`),
    nibbleRow("published", rightHex, "FNV-1a 64 offset basis"),
    hashRow("hash of 'a'", fnv1a([0x61], OFFSET_WRONG), "wrong — no published vector matches", false),
    hashRow("hash of 'a'", fnv1a([0x61], OFFSET_RIGHT), "correct", true),
    nibbleRow("published vector", hex16(PUBLISHED), "RFC / reference check value"),
  ];

  // Mark the offending digit in the first row and its fix in the second.
  const firstNibbles = rows[0].querySelectorAll(".hexnib");
  const secondNibbles = rows[1].querySelectorAll(".hexnib");
  if (digit >= 0) {
    firstNibbles[digit].classList.add("hexnib--diff");
    secondNibbles[digit].classList.add("hexnib--fixed");
  }

  container.replaceChildren(...rows);

  // A short, repeating sweep that draws the eye to the differing nibble without
  // animating anything that carries information.
  const sweep = document.createElement("p");
  sweep.className = "demo__foot";
  sweep.style.borderTop = "none";
  sweep.style.paddingTop = "0";
  sweep.innerHTML =
    "One digit. The multiplier was correct, the basis was not — and the test's " +
    "own first assertion repeated the same wrong constant, so the two agreed with " +
    "each other and only the <em>external</em> vectors disagreed.";
  container.append(sweep);
}