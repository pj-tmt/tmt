import { useState, type ReactNode } from 'react';
import { BrandMark } from '../src/brand-mark.js';
import { Button, Surface } from '../src/ui/index.js';
import { setReducedMotion, usePreview } from './preview.js';

/** What each component does when it moves (#5821), shown on its page with a replay. */
export const motionNotes: Record<string, string> = {
  alert: 'Enters with a short rise on the base duration.',
  button: 'Hover changes colour on fast; a press scales to 0.98 on fast.',
  checkbox: 'The tick draws in from left to right on base.',
  command: 'Rows change colour only; in a dialog it opens with the dialog.',
  dialog: 'The scrim fades; the dialog pops from 0.96 with a fade on base and leaves on fast.',
  'dropdown-menu': 'Fades in and rises from the side it opens toward on fast; leaves quicker.',
  popover: 'Fades in and rises from the side it opens toward on fast; leaves quicker.',
  select: 'Fades in and rises from the side it opens toward on fast; leaves quicker.',
  sheet: 'The scrim fades; the sheet slides from its edge on base and back on fast.',
  skeleton: 'A calm opacity pulse, never a shimmer sweep; still under reduced motion.',
  switch: 'The thumb slides on base while the track colour crossfades on fast.',
  tabs: 'The line underline slides to the selected tab on base.',
  tooltip: 'Fades in and rises from its side on fast; leaves quicker.',
};

/** Remounts its children, so a component's entrance plays again. */
export function Replay({ children }: { children: ReactNode }) {
  const [round, setRound] = useState(0);
  return (
    <div className="docs-replay">
      <div key={round}>{children}</div>
      <Button type="button" variant="outline" size="xs" onClick={() => setRound(round + 1)}>
        Replay
      </Button>
    </div>
  );
}

const durations = ['fast', 'base'] as const;
const easings = ['ease-out', 'ease-in'] as const;

function Demo({ duration, easing }: { duration: string; easing: string }) {
  const [moved, setMoved] = useState(false);
  return (
    <li className="docs-motion-demo">
      <span className="docs-motion-label">
        <code>{duration}</code> + <code>{easing}</code>
      </span>
      <span className="docs-motion-track">
        <span
          className="docs-motion-box"
          data-moved={moved || undefined}
          style={{
            transitionDuration: `var(--tmt-ui-motion-${duration})`,
            transitionTimingFunction: `var(--tmt-ui-motion-${easing})`,
          }}
        />
      </span>
      <Button type="button" variant="outline" size="xs" onClick={() => setMoved(!moved)}>
        Replay
      </Button>
    </li>
  );
}

export function MotionPage() {
  const { reduced } = usePreview();
  return (
    <>
      <div className="docs-reduced">
        <Button
          type="button"
          variant={reduced ? 'secondary' : 'outline'}
          size="sm"
          aria-pressed={reduced}
          onClick={() => setReducedMotion(!reduced)}
        >
          {reduced ? 'Reduced motion: on' : 'Simulate reduced motion'}
        </Button>
        <span className="docs-muted">
          Every duration becomes 0ms, as under prefers-reduced-motion.
        </span>
      </div>
      <h2>Durations and easings</h2>
      <p>Press Replay to move each box with that pair.</p>
      <ul className="docs-motion-demos">
        {durations.flatMap((duration) =>
          easings.map((easing) => (
            <Demo key={`${duration}-${easing}`} duration={duration} easing={easing} />
          )),
        )}
      </ul>
      <h2>Rules</h2>
      <ul>
        <li>
          Entering takes base with ease-out; leaving takes fast with ease-in, so exits are quicker.
        </li>
        <li>
          Floating layers fade and rise 6px from the side they open toward; a dialog pops from 0.96;
          a sheet slides from its edge.
        </li>
        <li>Controls answer at once: colour on fast, a press at most 0.98.</li>
        <li>
          Nothing moves on its own except a skeleton&apos;s calm pulse. Rows, links and lists change
          colour only.
        </li>
        <li>Under reduced motion every duration is 0ms and the pulse holds still.</li>
        <li>
          Durations and easings come from the motion tokens only; a policy test rejects literals.
        </li>
      </ul>
    </>
  );
}

const levels = [
  ['paper', 'The page.', 'elevation-0'],
  ['sheet', 'A card or panel.', 'elevation-1'],
  ['raised', 'A surface while hovered or dragged.', 'elevation-2'],
  ['overlay', 'Popover, menu, Select, tooltip.', 'elevation-2'],
  ['modal', 'Dialog and sheet.', 'elevation-3'],
] as const;

export function ElevationPage() {
  return (
    <>
      <p>
        Five surface levels on one elevation scale. Each level pairs a fill, a hairline edge and a
        shadow: dark themes show shadows weakly, so the edge carries the level there.
      </p>
      <div className="docs-canvas docs-elevation">
        {levels.map(([level, use, shadow], index) => (
          <Surface
            key={level}
            level={level}
            className="docs-elevation-level"
            style={{ marginLeft: `calc(var(--tmt-ui-ref-space-5) * ${index})` }}
          >
            <b>{level}</b>
            <span className="docs-muted">
              {use} <code>{shadow}</code>
            </span>
          </Surface>
        ))}
      </div>
      <h2>Which part sits where</h2>
      <ul>
        <li>Card: sheet.</li>
        <li>Popover, Dropdown, Select, Command in a popover, Tooltip: overlay.</li>
        <li>Dialog and Sheet: modal.</li>
      </ul>
      <p>
        One shadow token per level, so a change to <code>elevation-2</code> changes every floating
        layer at once.
      </p>
    </>
  );
}

/** The two B mark options on /docs/brand, at the sizes the product uses. */
export function BrandPage() {
  return (
    <>
      <p>
        Under palette B the mark is red, black and white. Two options for the glyph on dark; the
        lead will ask Ben to pick one. Everything else in the docs uses option A until then.
      </p>
      {(['a', 'b'] as const).map((option) => (
        <section key={option} className="docs-brand-option" data-tmt-mark={option}>
          <h2>{option === 'a' ? 'A · white glyph on red' : 'B · red glyph on black'}</h2>
          <div className="docs-canvas docs-brand-row">
            {[16, 24, 32, 48].map((size) => (
              <span key={size} className="docs-brand-size">
                <BrandMark size={size} />
                <span className="docs-muted">{size}px</span>
              </span>
            ))}
          </div>
        </section>
      ))}
    </>
  );
}
