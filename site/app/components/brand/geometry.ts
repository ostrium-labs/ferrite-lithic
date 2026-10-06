/** Verified Edge: 24-module field, two-unit trace, four-unit square terminals. */
export const MARK = {
  viewBox: '0 0 24 24',
  branch: 'M20 4 H13 L10 7 V17 L13 20 H20',
  input: { x: 2, y: 11, width: 8, height: 2 },
  upper: { x: 18, y: 2, width: 4, height: 4 },
  lower: { x: 18, y: 18, width: 4, height: 4 },
} as const;
/** Same skeleton snapped to whole pixels, with thicker terminals at 16px. */
export const MICRO_MARK = {
  viewBox: '0 0 16 16',
  branch: 'M13 3 H7 V13 H13',
  input: { x: 1, y: 7, width: 6, height: 2 },
  upper: { x: 12, y: 1, width: 3, height: 3 },
  lower: { x: 12, y: 12, width: 3, height: 3 },
} as const;
