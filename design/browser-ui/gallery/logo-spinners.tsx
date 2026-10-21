import { useId, type ReactNode } from 'react';
import { brandMarkSmallShape as shape } from '../src/brand-mark-svg.js';
import { Cell, Cells, Pair } from './kit.js';
import './logo-spinners.css';

/**
 * Gallery-only candidates for a busy mark drawn from the logo: the small form of the 04D-1 mark
 * (flat panel, aurora beam, chevron prompt) with one moving part each. Plain SVG and CSS keyframes
 * on the shared --tmt-ui-spin-duration; colour comes from the mark's brand roles, so both themes
 * follow, and `mono` paints everything in currentColor for inline text. The still frame is the
 * stylesheet's base state: motion is only added when the viewer has not asked for less, so a
 * reduced-motion viewer (or `still`) sees a frame that already reads as the mark.
 */
export type LogoSpin = 'aurora' | 'caret' | 'edge' | 'panel';

function LogoSpinner({
  kind,
  size,
  mono = false,
  still = false,
}: {
  kind: LogoSpin;
  size: number;
  mono?: boolean;
  still?: boolean;
}) {
  const id = useId().replace(/:/g, '');
  const terminal = kind === 'panel';
  const mark = (
    <>
      {!terminal && (
        <path className="logo-beam" d={shape.beam} fill={mono ? undefined : `url(#${id}-aurora)`} />
      )}
      <path className="logo-panel" d={shape.panel} fill={mono ? undefined : `url(#${id}-panel)`} />
      <path className="logo-prompt" d={shape.prompt} />
    </>
  );
  return (
    <span
      className="logo-spin"
      data-kind={kind}
      data-mono={mono || undefined}
      data-still={still || undefined}
      aria-hidden="true"
    >
      {/* The terminal alone is the left half of the mark: centre the panel in the square. */}
      <svg viewBox={terminal ? '-10.5 11 78 78' : '0 0 96 96'} width={size} height={size}>
        <defs>
          <linearGradient id={`${id}-aurora`} x1="0.2" y1="0.05" x2="0.85" y2="1">
            <stop className="tmt-brand-mark-cyan" offset="0" />
            <stop className="tmt-brand-mark-green" offset="0.5" />
            <stop className="tmt-brand-mark-gold" offset="1" />
          </linearGradient>
          <linearGradient id={`${id}-panel`} x1="0" y1="0" x2="0.6" y2="1">
            <stop className="tmt-brand-mark-panel-top" offset="0" />
            <stop className="tmt-brand-mark-panel-bottom" offset="1" />
          </linearGradient>
          <clipPath id={`${id}-panel-clip`}>
            <path d={shape.panel} />
          </clipPath>
          <clipPath id={`${id}-clip`}>
            <path d={shape.beam} />
            <path d={shape.panel} />
          </clipPath>
          <filter id={`${id}-glow`} x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="5" />
          </filter>
        </defs>
        {kind === 'aurora' && (
          <>
            {mark}
            <g clipPath={`url(#${id}-clip)`}>
              <rect className="logo-band" x="-26" y="-4" width="22" height="104" />
            </g>
          </>
        )}
        {kind === 'caret' && (
          <>
            {mark}
            <circle className="logo-glow" cx="40" cy="64" r="13" filter={`url(#${id}-glow)`} />
            <path className="logo-caret" d="M34 64 L46 64" />
          </>
        )}
        {kind === 'panel' && (
          <>
            {mark}
            {/* Inside the panel, so the light reads on a dark panel in either theme. */}
            <g clipPath={`url(#${id}-panel-clip)`}>
              <path className="logo-trail" d={shape.panel} pathLength="100" />
              <path className="logo-edge" d={shape.panel} pathLength="100" />
            </g>
          </>
        )}
        {kind === 'edge' && (
          <>
            {mark}
            <path className="logo-trail" d={shape.panel} pathLength="100" />
            <path className="logo-edge" d={shape.panel} pathLength="100" />
          </>
        )}
      </svg>
    </span>
  );
}

const SIZES = [16, 20, 24] as const;

const candidates: readonly (readonly [LogoSpin, string, ReactNode])[] = [
  [
    'aurora',
    'L1: the aurora band sweeps across the panel',
    'A soft band of light crosses the mark left to right, clipped to the panel and the beam. Still frame: the band rests across the middle.',
  ],
  [
    'caret',
    'L2: the prompt caret pulses with a soft glow',
    'The cursor after the chevron breathes: it dims and returns while its glow swells. Still frame: caret lit, glow half.',
  ],
  [
    'edge',
    'L3: a light travels the panel edge',
    'A short bright arc with a fading trail runs round the rounded panel once per cycle. Still frame: the arc rests at the top corner.',
  ],
  [
    'panel',
    'L3b: terminal only, a light travels its edge',
    'Ben: only the inner terminal panel. The panel and its chevron stand alone, with no beam, glow, tile or aurora, and the light runs round its rounded edge. Still frame: the arc rests at the top.',
  ],
];

/** Each candidate at 16, 20 and 24 px in colour and mono, in light and dark, with its still frame. */
export function LogoSpinners() {
  return (
    <>
      <p className="gallery-note">
        Logo-based candidates beside a1-a3: the same 0.9 s cycle (--tmt-ui-spin-duration), plain SVG
        and CSS, brand-role colour in both themes, a mono variant for inline text, and a
        reduced-motion still frame (also shown as "still").
      </p>
      {candidates.map(([kind, title, note]) => (
        <Pair key={kind} title={title} note={note}>
          {() => (
            <Cells>
              {SIZES.map((size) => (
                <Cell key={size} label={`${size}px`}>
                  <LogoSpinner kind={kind} size={size} />
                </Cell>
              ))}
              {SIZES.map((size) => (
                <Cell key={`mono-${size}`} label={`mono ${size}px`}>
                  <LogoSpinner kind={kind} size={size} mono />
                </Cell>
              ))}
              <Cell label="in text">
                <span className="logo-inline">
                  Saving <LogoSpinner kind={kind} size={16} mono /> draft
                </span>
              </Cell>
              {SIZES.map((size) => (
                <Cell key={`still-${size}`} label={`still ${size}px`}>
                  <LogoSpinner kind={kind} size={size} still />
                </Cell>
              ))}
              <Cell label="still mono 16px">
                <LogoSpinner kind={kind} size={16} mono still />
              </Cell>
            </Cells>
          )}
        </Pair>
      ))}
    </>
  );
}
