import type { ReactNode } from 'react';
import { BrowserAction } from '../src/react.js';
import { Cell, Cells, Pair, noop } from './kit.js';
import './spinner-variants.css';

/**
 * Gallery-only refinements of candidate a (the braille cycle). Each mark is a 1em square of SVG
 * dots in currentColor, wrapped in a span: the action mark's legacy rule turns a direct <svg>
 * child, so the span keeps these from rotating. Dots are drawn, not typeset, so they stay crisp at
 * 12-16 px in any font.
 */

/** The 2x4 braille cell has eight dots; the sweep walks the ring around its edge. */
const sweepDots: readonly (readonly [number, number])[] = [
  [0, 0],
  [1, 0],
  [1, 1],
  [1, 2],
  [1, 3],
  [0, 3],
  [0, 2],
  [0, 1],
];
/** The 2x3 snake's six dots are all on the ring. */
const snakeDots: readonly (readonly [number, number])[] = [
  [0, 0],
  [1, 0],
  [1, 1],
  [1, 2],
  [0, 2],
  [0, 1],
];

function dotsOf(
  kind: 'sweep' | 'snake',
  cells: readonly (readonly [number, number])[],
  x: (column: number) => number,
  y: (row: number) => number,
  radius: number,
) {
  return cells.map(([column, row], index) => (
    <circle
      key={index}
      className={`variant-dot variant-${kind}-dot`}
      cx={x(column)}
      cy={y(row)}
      r={radius}
      style={{ '--i': index / cells.length } as React.CSSProperties}
    />
  ));
}

/** (a1) Eight dots, one bright head and a fading trail, one lap per duration. */
const sweep = (
  <span className="variant variant-sweep" aria-hidden="true">
    <svg viewBox="0 0 16 16">
      {dotsOf(
        'sweep',
        sweepDots,
        (c) => 5.5 + c * 5,
        (r) => 2.5 + r * 3.67,
        1.3,
      )}
    </svg>
  </span>
);

/** (a2) Six dots, three lit, eased in and out so the snake gathers and lets go. */
const snake = (
  <span className="variant variant-snake" aria-hidden="true">
    <svg viewBox="0 0 16 16">
      {dotsOf(
        'snake',
        snakeDots,
        (c) => 5 + c * 6,
        (r) => 3 + r * 5,
        1.6,
      )}
    </svg>
  </span>
);

/** (a3) Three dots rising in turn, the rhythm of the >_ prompt: step, step, rest. */
const wave = (
  <span className="variant variant-wave" aria-hidden="true">
    <svg viewBox="0 0 16 16">
      {[0, 1, 2].map((index) => (
        <circle
          key={index}
          className="variant-dot variant-wave-dot"
          cx={3.5 + index * 4.5}
          cy={9}
          r={1.6}
          style={{ '--i': index * 0.16 } as React.CSSProperties}
        />
      ))}
    </svg>
  </span>
);

const variants: readonly (readonly [string, ReactNode])[] = [
  ['a1: eight-dot braille sweep, fading trail', sweep],
  ['a2: 2x3 dot snake, eased', snake],
  ['a3: three-dot wave, the >_ rhythm', wave],
];

/** The refined variants of a, each inside the real busy action in light and dark, plus 12/16/32 px. */
export function SpinnerVariants() {
  return (
    <>
      {variants.map(([title, mark]) => (
        <Pair key={title} title={title}>
          {() => (
            <Cells>
              {(['primary', 'text', 'destructive'] as const).map((variant) => (
                <Cell key={variant} label={variant}>
                  <BrowserAction
                    type="button"
                    variant={variant}
                    label={variant === 'destructive' ? 'Delete page' : 'Save draft'}
                    busy
                    busyMark={mark}
                    onActivate={noop}
                  />
                </Cell>
              ))}
              {[12, 16, 32].map((size) => (
                <Cell key={size} label={`${size}px`}>
                  <span className="variant-size" style={{ fontSize: size }}>
                    {mark}
                  </span>
                </Cell>
              ))}
            </Cells>
          )}
        </Pair>
      ))}
    </>
  );
}
