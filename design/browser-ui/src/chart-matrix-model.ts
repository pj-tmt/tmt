import { compactCount, level } from './chart-math.js';

export interface MatrixParticipant {
  id: string;
  label: string;
}

export interface MatrixOptions {
  title: string;
  /** The participants, as senders (rows) and recipients (columns) alike. */
  participants: readonly MatrixParticipant[];
  /** Messages sent `from` to `to`: null or undefined is unavailable, 0 is a known none. */
  value: (from: string, to: string) => number | null | undefined;
  /** A trailing column with what each row sent to the others; `—` when nothing is observed. */
  sum?: boolean;
  empty?: string;
}

export interface MatrixCell {
  /** What the cell reads: a count, `·` (observed none), `—` (unavailable) or blank (self). */
  text: string;
  /** Intensity step 1..LEVELS; 0 or absent paints no fill. */
  level: number;
  self: boolean;
  label: string;
}

export interface MatrixModel {
  columns: string[];
  rows: { label: string; cells: MatrixCell[]; sum?: string }[];
}

/** The matrix as plain data: every cell's text, intensity and spoken label. */
export function matrixModel(options: MatrixOptions): MatrixModel {
  const people = options.participants;
  let max = 0;
  for (const from of people)
    for (const to of people)
      if (from.id !== to.id) max = Math.max(max, options.value(from.id, to.id) ?? 0);
  return {
    columns: [...people.map((person) => person.label), ...(options.sum ? ['sum'] : [])],
    rows: people.map((from) => {
      let sent: number | null = null;
      const cells = people.map((to): MatrixCell => {
        if (from.id === to.id) {
          return { text: '', level: 0, self: true, label: `${from.label} to itself` };
        }
        const count = options.value(from.id, to.id);
        if (count != null) sent = (sent ?? 0) + count;
        return {
          text: count == null ? '—' : count === 0 ? '·' : compactCount(count),
          level: level(count, max) ?? 0,
          self: false,
          label: `${from.label} to ${to.label}: ${count == null ? 'unavailable' : count}`,
        };
      });
      return {
        label: from.label,
        cells,
        sum: options.sum ? (sent == null ? '—' : compactCount(sent)) : undefined,
      };
    }),
  };
}
