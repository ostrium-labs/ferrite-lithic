/**
 * Two bit-level demos, one model.
 *
 * Both a DEFLATE bit reader and a canonical Huffman decoder hold a small register
 * that bits arrive into at one end and leave from at the other. The difference is
 * what happens at the leaving end: DEFLATE hands out single bits, Huffman peeks nine
 * of them and reverses them into a table index.
 *
 * Both are animated from the same constants the Rust designs use, including the
 * nine-cycles-per-byte rate that the DEFLATE module documents and measures.
 */

import { MONO, palette, roundRect } from "../lib/ticker";

/** RFC 1951's fixed literal/length code lengths, transcribed from the RFC. */
function fixedLiteralLengthLengths(): number[] {
  const lengths: number[] = [];
  for (let symbol = 0; symbol < 288; symbol += 1) {
    if (symbol < 144) lengths.push(8);
    else if (symbol < 256) lengths.push(9);
    else if (symbol < 280) lengths.push(7);
    else lengths.push(8);
  }
  return lengths;
}

/** Canonical assignment: sort by (length, symbol), hand out codes in that order. */
function canonicalCodes(lengths: number[]): { code: number; length: number }[] {
  const codes: { code: number; length: number }[] = lengths.map((length) => ({ code: 0, length }));
  let code = 0;
  for (let length = 1; length <= 15; length += 1) {
    for (let symbol = 0; symbol < lengths.length; symbol += 1) {
      if (lengths[symbol] !== length) continue;
      codes[symbol] = { code, length };
      code += 1;
    }
    code <<= 1;
  }
  return codes;
}

/** A bit sequence in DEFLATE's order: bit 0 of each byte first. */
export function toBits(bytes: number[]): boolean[] {
  const bits: boolean[] = [];
  for (const byte of bytes) {
    for (let position = 0; position < 8; position += 1) {
      bits.push(((byte >> position) & 1) === 1);
    }
  }
  return bits;
}

function label(
  context: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  color: string,
  size = 12,
  align: CanvasTextAlign = "left",
): void {
  context.font = `${size}px ${MONO}`;
  context.fillStyle = color;
  context.textAlign = align;
  context.textBaseline = "middle";
  context.fillText(text, x, y);
}

/* ------------------------------------------------------------------ DEFLATE */

const DEFLATE_CYCLES_PER_BYTE = 9;

/**
 * The bit reader, animated: a staging register, a bit out per cycle, and a new byte
 * only when the register is empty — which is why the rate is nine cycles and not eight.
 */
export function drawBitStream(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  elapsed: number,
): void {
  context.fillStyle = palette.bg;
  context.fillRect(0, 0, width, height);

  // A real DEFLATE stream: a stored block header, LEN and the first payload bytes.
  const bytes = [0x01, 0x05, 0x00, 0xfa, 0xff, 0x48, 0x65, 0x6c, 0x6c, 0x6f];
  const cycle = Math.floor(elapsed / 90) % (bytes.length * DEFLATE_CYCLES_PER_BYTE);

  const loadedByte = Math.min(bytes.length - 1, Math.floor(cycle / DEFLATE_CYCLES_PER_BYTE));
  const withinByte = cycle % DEFLATE_CYCLES_PER_BYTE;
  const bitsLeft = withinByte === 0 ? 0 : 8 - withinByte + 1;

  // The staging register, drawn as the byte with its bit index marked.
  const regX = 34;
  const regY = height / 2 - 26;
  const cell = 30;

  label(context, "staging register", regX, regY - 26, palette.inkFaint, 11);

  const byte = bytes[loadedByte] ?? 0;
  for (let position = 7; position >= 0; position -= 1) {
    const x = regX + (7 - position) * (cell + 6);
    const isOut = position >= bitsLeft;
    const bit = ((byte >> position) & 1) === 1;

    context.fillStyle = isOut ? palette.panel : bit ? palette.accent : palette.line;
    roundRect(context, x, regY, cell, cell, 5);
    context.fill();

    if (!isOut) {
      context.strokeStyle = bit ? palette.accent : palette.lineSoft;
      context.lineWidth = 1.5;
      roundRect(context, x, regY, cell, cell, 5);
      context.stroke();
    }

    label(
      context,
      isOut ? "·" : String(position),
      x + cell / 2,
      regY + cell / 2,
      isOut ? palette.inkFaint : bit ? palette.bg : palette.inkDim,
      13,
      "center",
    );
  }

  // The bit currently leaving. It leaves from the *right* end, because the register is
  // drawn with bit 7 on the left and bit 0 on the right and the design peels bit 0
  // first: an arrow on the far side would be a picture of the opposite order.
  const regRight = regX + 8 * (cell + 6) - 6;
  const pulse = withinByte === 0 ? 0 : 1 - (withinByte - 1) / DEFLATE_CYCLES_PER_BYTE;
  const outBit = bitsLeft < 8 ? ((byte >> bitsLeft) & 1) === 1 : false;

  context.strokeStyle = palette.accent2;
  context.globalAlpha = 0.25 + pulse * 0.6;
  context.lineWidth = 2;
  context.beginPath();
  context.moveTo(regRight + 8, regY + cell / 2);
  context.lineTo(regRight + 34, regY + cell / 2);
  context.stroke();
  context.globalAlpha = 1;

  // The bit itself, travelling.
  context.fillStyle = pulse > 0.05 ? palette.accent : palette.line;
  roundRect(context, regRight + 40, regY + cell / 2 - 11, 22, 22, 5);
  context.fill();
  label(
    context,
    withinByte === 0 ? "" : outBit ? "1" : "0",
    regRight + 51,
    regY + cell / 2,
    withinByte === 0 ? palette.inkFaint : pulse > 0.05 ? palette.bg : palette.inkFaint,
    13,
    "center",
  );
  label(context, "bit out", regRight + 40, regY + cell / 2 + 26, palette.accent2, 10);

  // The stream itself, one box per byte, filling in as it is consumed.
  const streamY = height - 74;
  label(context, "stream, least significant bit of each byte first", regX, streamY - 24, palette.inkFaint, 11);

  for (let index = 0; index < bytes.length; index += 1) {
    const x = regX + index * 20;
    const loaded = index <= loadedByte;
    context.fillStyle = loaded ? palette.panel : palette.bg;
    context.strokeStyle = index === loadedByte ? palette.accent : palette.line;
    context.lineWidth = index === loadedByte ? 1.5 : 1;
    roundRect(context, x, streamY, 16, 34, 4);
    context.fill();
    context.stroke();

    if (loaded) {
      // Fill from the low bit upwards: the order the reader takes them in.
      const emitted = index < loadedByte ? 8 : Math.max(0, 8 - bitsLeft);
      for (let bit = 0; bit < emitted; bit += 1) {
        const on = ((bytes[index] >> bit) & 1) === 1;
        context.fillStyle = on ? palette.accent : palette.inkFaint;
        context.fillRect(x + 2, streamY + 32 - bit * 4, 12, 3);
      }
    }
  }

  // The readout.
  const panelX = width - 210;
  context.fillStyle = palette.panel;
  context.strokeStyle = palette.line;
  context.lineWidth = 1;
  roundRect(context, panelX, 24, 186, 96, 8);
  context.fill();
  context.stroke();

  label(context, "cycle", panelX + 14, 44, palette.inkFaint, 11);
  label(context, String(cycle), panelX + 172, 44, palette.ink, 13, "right");
  label(context, "bits_left", panelX + 14, 66, palette.inkFaint, 11);
  label(
    context,
    String(bitsLeft),
    panelX + 172,
    66,
    bitsLeft === 0 ? palette.accent : palette.ink,
    13,
    "right",
  );
  label(context, "in_ready", panelX + 14, 88, palette.inkFaint, 11);
  label(
    context,
    bitsLeft === 0 ? "1" : "0",
    panelX + 172,
    88,
    bitsLeft === 0 ? palette.good : palette.inkFaint,
    13,
    "right",
  );

  label(
    context,
    "nine cycles per byte: eight to shift the bits out, one to take the next byte",
    regX,
    height - 16,
    palette.inkFaint,
    11,
  );
}

/* ------------------------------------------------------------------ Huffman */

const TABLE_BITS = 9;
const TABLE_ENTRIES = 1 << TABLE_BITS;

/** The 512-entry decode table, exactly as `Table::canonical` builds it. */
function decodeTable(): { symbol: number; length: number }[] {
  const codes = canonicalCodes(fixedLiteralLengthLengths());
  const entries = Array.from({ length: TABLE_ENTRIES }, () => ({ symbol: 0, length: 0 }));
  for (const [symbol, word] of codes.entries()) {
    if (word.length === 0) continue;
    const span = 1 << (TABLE_BITS - word.length);
    const first = word.code << (TABLE_BITS - word.length);
    for (let index = first; index < first + span; index += 1) {
      entries[index] = { symbol, length: word.length };
    }
  }
  return entries;
}

/** The message being decoded, and its bit stream: each code top bit first. */
function huffmanMessage(): { symbols: number[]; bits: boolean[] } {
  const codes = canonicalCodes(fixedLiteralLengthLengths());
  const symbols = [72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 33];
  const bits: boolean[] = [];
  for (const symbol of symbols) {
    const word = codes[symbol];
    for (let position = word.length - 1; position >= 0; position -= 1) {
      bits.push(((word.code >> position) & 1) === 1);
    }
  }
  return { symbols, bits };
}

export function drawHuffman(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  elapsed: number,
): void {
  context.fillStyle = palette.bg;
  context.fillRect(0, 0, width, height);

  const table = decodeTable();
  const { bits } = huffmanMessage();

  // One symbol every 900ms, with the lookup landing in the middle of its window so the
  // nine-bit peek is visible before the answer is.
  const window = 900;
  const step = Math.floor(elapsed / window) % bits.length;
  const phase = (elapsed % window) / window;

  const peekStart = Math.min(step, Math.max(0, bits.length - 1));
  const peekBits: boolean[] = [];
  for (let offset = 0; offset < TABLE_BITS; offset += 1) {
    peekBits.push(bits[(peekStart + offset) % bits.length]);
  }

  // Stream order: bit 0 is the code's most significant bit, because a Huffman code is
  // written top-down into a stream that arrives bottom-up. The reversal is the whole
  // content of this lookup.
  let index = 0;
  for (let position = 0; position < TABLE_BITS; position += 1) {
    index = (index << 1) | (peekBits[position] ? 1 : 0);
  }
  const entry = table[index];

  // ---- the arriving bits
  const streamY = 46;
  label(context, "bit buffer, arriving at the low end", 30, streamY - 22, palette.inkFaint, 11);

  const cell = 34;
  const gap = 6;
  for (let position = 0; position < TABLE_BITS; position += 1) {
    const x = 30 + position * (cell + gap);
    const bit = peekBits[position];
    const filled = phase > position / (TABLE_BITS + 4);

    context.fillStyle = filled ? (bit ? palette.accent : palette.line) : palette.panel;
    roundRect(context, x, streamY, cell, cell, 5);
    context.fill();
    context.strokeStyle = palette.line;
    context.lineWidth = 1;
    context.stroke();

    label(
      context,
      filled ? (bit ? "1" : "0") : "",
      x + cell / 2,
      streamY + cell / 2,
      bit ? palette.bg : palette.inkDim,
      14,
      "center",
    );
    label(context, `b${position}`, x + cell / 2, streamY + cell + 14, palette.inkFaint, 9, "center");
  }

  // ---- the reversal
  const arrowY = streamY + cell + 44;
  context.strokeStyle = palette.accent3;
  context.lineWidth = 1.5;
  context.setLineDash([4, 4]);
  context.beginPath();
  context.moveTo(30 + cell / 2, arrowY - 16);
  context.lineTo(30 + (TABLE_BITS - 1) * (cell + gap) + cell / 2, arrowY - 16);
  context.stroke();
  context.setLineDash([]);
  label(
    context,
    "reverse: b0 is the code's most significant bit",
    30,
    arrowY,
    palette.accent3,
    11,
  );

  // ---- the index, high bit first
  const indexY = arrowY + 34;
  const indexCell = 30;
  for (let position = TABLE_BITS - 1; position >= 0; position -= 1) {
    const x = 30 + (TABLE_BITS - 1 - position) * (indexCell + 5);
    const bit = ((index >> position) & 1) === 1;
    context.fillStyle = bit ? palette.accent2 : palette.line;
    roundRect(context, x, indexY, indexCell, 26, 4);
    context.fill();
    label(context, bit ? "1" : "0", x + indexCell / 2, indexY + 13, bit ? palette.bg : palette.inkDim, 12, "center");
  }
  label(context, "table index", 30 + (TABLE_BITS * (indexCell + 5)) + 14, indexY + 13, palette.inkFaint, 11);

  // ---- the table, with the selected row lit
  const tableX = width - 232;
  const tableY = 40;
  const rowsShown = 9;
  const rowH = 18;
  label(context, "512-entry decode table", tableX, tableY - 16, palette.inkFaint, 11);

  const firstRow = Math.max(0, Math.min(index - 4, TABLE_ENTRIES - rowsShown));
  for (let row = 0; row < rowsShown; row += 1) {
    const rowIndex = firstRow + row;
    const hit = rowIndex === index;
    const y = tableY + row * rowH;
    context.fillStyle = hit ? palette.accent : palette.panel;
    roundRect(context, tableX, y, 200, rowH - 3, 3);
    context.fill();
    label(context, String(rowIndex), tableX + 8, y + (rowH - 3) / 2, hit ? palette.bg : palette.inkFaint, 10);
    label(
      context,
      table[rowIndex].length === 0 ? "no code" : `sym ${table[rowIndex].symbol}`,
      tableX + 52,
      y + (rowH - 3) / 2,
      hit ? palette.bg : palette.inkDim,
      10,
    );
    label(
      context,
      table[rowIndex].length === 0 ? "" : `${table[rowIndex].length} bits`,
      tableX + 192,
      y + (rowH - 3) / 2,
      hit ? palette.bg : palette.inkFaint,
      10,
      "right",
    );
  }

  // ---- the answer
  const answerY = indexY + 66;
  const answered = phase > 0.62;
  context.strokeStyle = answered ? palette.good : palette.line;
  context.lineWidth = 1.5;
  roundRect(context, 30, answerY, 300, 44, 8);
  context.stroke();

  label(context, "out_valid", 44, answerY + 15, palette.inkFaint, 10);
  label(context, answered ? "1" : "0", 108, answerY + 15, answered ? palette.good : palette.inkFaint, 12, "right");

  label(context, "symbol", 150, answerY + 15, palette.inkFaint, 10);
  label(
    context,
    answered ? String(entry.symbol) : "—",
    214,
    answerY + 15,
    answered ? palette.accent : palette.inkFaint,
    12,
    "right",
  );

  label(context, "code_len", 236, answerY + 15, palette.inkFaint, 10);
  label(
    context,
    answered ? String(entry.length) : "—",
    316,
    answerY + 15,
    answered ? palette.accent : palette.inkFaint,
    12,
    "right",
  );

  label(
    context,
    `fixed literal/length code from RFC 1951 §3.2.6 · entry ${index}`,
    30,
    height - 16,
    palette.inkFaint,
    11,
  );
}