import { compactCount, level } from './chart-math.js';

/** Intensity steps above empty in a strip: five swatches with the empty one, like a contribution graph. */
export const STRIP_STEPS = 4;

export interface StripRow {
  id: string;
  label: string;
  /** One observation per slot, oldest first: null is not observed, 0 is a quiet slot. */
  slots: readonly (number | null)[];
}

export interface StripsOptions {
  title: string;
  rows: readonly StripRow[];
  /** Squares per row; keep it at or under about 48. */
  slotCount: number;
  /** The time range a slot covers, for its tip ("14:00–15:00"). */
  slotRange: (index: number) => string;
  /** How a slot's value reads in the tip; default is a compact count. */
  format?: (value: number) => string;
  /** The words under the strip's two ends. */
  ends: readonly [string, string];
  empty?: string;
  /** False when the caller places `stripsLegend()` itself (a card header); default true. */
  legend?: boolean;
}

export interface StripSlot {
  /** Intensity step 1..STRIP_STEPS, 0 for a known quiet slot, null for not observed. */
  level: number | null;
  /** What the tip says: the seat, the time range and the value. */
  tip: { seat: string; range: string; value: string };
  /** The same as one sentence, for assistive technology. */
  label: string;
}

export interface StripsModel {
  rows: { label: string; summary: string; slots: StripSlot[] }[];
}

/**
 * Each row scaled to its own busiest slot, so a quiet seat still shows its pattern. An unknown
 * slot stays hollow and a known quiet one is the empty step: no data never reads as silence.
 */
export function stripsModel(options: StripsOptions): StripsModel {
  const format = options.format ?? compactCount;
  return {
    rows: options.rows.map((row) => {
      const max = Math.max(0, ...row.slots.map((v) => v ?? 0));
      const active = row.slots.filter((v) => (v ?? 0) > 0).length;
      const known = row.slots.filter((v) => v != null).length;
      return {
        label: row.label,
        summary: `${row.label}: active in ${active} of ${known} observed slots`,
        slots: Array.from({ length: options.slotCount }, (_, at) => {
          const value = row.slots[at];
          const range = options.slotRange(at);
          const text = value == null ? 'not observed' : `${format(value)} tokens`;
          return {
            level: level(value, max, STRIP_STEPS),
            tip: { seat: row.label, range, value: text },
            label: `${row.label}, ${range}: ${text}`,
          };
        }),
      };
    }),
  };
}
