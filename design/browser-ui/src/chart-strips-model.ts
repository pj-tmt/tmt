import { level } from './chart-math.js';

export interface StripRow {
  id: string;
  label: string;
  /** One observation per slot, oldest first: null is not observed, 0 is a quiet slot. */
  slots: readonly (number | null)[];
}

export interface StripsOptions {
  title: string;
  rows: readonly StripRow[];
  /** 24 or 48 slots per row. */
  slotCount: 24 | 48;
  /** A name for each slot (its time), for the slot's tooltip. */
  slotLabel: (index: number) => string;
  /** The words under the strip's two ends. */
  ends: readonly [string, string];
  empty?: string;
}

export interface StripSlot {
  /** Intensity step 1..LEVELS, 0 for a known quiet slot, null for not observed. */
  level: number | null;
  title: string;
}

export interface StripsModel {
  rows: { label: string; summary: string; slots: StripSlot[] }[];
}

/**
 * Each row scaled to its own busiest slot, so a quiet seat still shows its pattern. An unknown
 * slot stays blank and a known quiet one is the faintest step: no data never reads as silence.
 */
export function stripsModel(options: StripsOptions): StripsModel {
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
          return {
            level: level(value, max),
            title: `${options.slotLabel(at)}: ${value == null ? 'not observed' : value}`,
          };
        }),
      };
    }),
  };
}
