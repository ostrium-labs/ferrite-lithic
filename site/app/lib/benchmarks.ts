/** Recorded speed.rs output. The clock is an assumption, not a measured silicon frequency. */
export const BENCHMARK_CLOCK_GHZ = 1;
export const SPEED_RESULTS = [
  { design: "crc32", reference: "crc32fast", cyclesPerByte: 1, softwareNs: 0.116 },
  { design: "crc3", reference: "crc", cyclesPerByte: 1, softwareNs: 2.24 },
  { design: "hex", reference: "hex", cyclesPerByte: 3, softwareNs: 4.14 },
  { design: "base64", reference: "base64", cyclesPerByte: 5.33, softwareNs: 0.39 },
  { design: "memchr", reference: "memchr", cyclesPerByte: 1.01, softwareNs: 0.24 },
] as const;

export function speedVerdict(result: (typeof SPEED_RESULTS)[number]) {
  const designNs = result.cyclesPerByte / BENCHMARK_CLOCK_GHZ;
  const hardwareWins = designNs < result.softwareNs;
  const ratio = hardwareWins ? result.softwareNs / designNs : designNs / result.softwareNs;
  return `${hardwareWins ? "design" : "software"} ${ratio.toFixed(1)}× faster`;
}
