import { BrowserAction, BusyMark } from '../src/react.js';
import { Cell, Cells, Pair, Section, noop } from './kit.js';
import { LogoSpinners } from './logo-spinners.js';
import { SpinnerVariants } from './spinner-variants.js';
import './spinner-candidates.css';

const candidates = [
  ['current: braille cycle (Ops TUI), the shipped BusyMark', <BusyMark key="busy" />],
  [
    'b: 2x2 grid',
    <span key="grid" className="candidate candidate-grid">
      <span />
      <span />
      <span />
      <span />
    </span>,
  ],
  [
    'c: square with a travelling segment',
    <span key="square" className="candidate candidate-square">
      <span />
    </span>,
  ],
] as const;

/** Gallery-only: the current busy mark and each candidate inside the real busy action, all three variants. */
export function SpinnerCandidates() {
  return (
    <Section id="spinner-candidates">
      <p className="gallery-note">
        The current mark is the shipped BusyMark (spinner.css). The others are candidates for
        refined variants; every one uses --tmt-ui-spin-duration and stands still under reduced
        motion.
      </p>
      {candidates.map(([title, mark]) => (
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
              <Cell label="32px">
                <span className="gallery-candidate-large">{mark}</span>
              </Cell>
            </Cells>
          )}
        </Pair>
      ))}
      <SpinnerVariants />
      <LogoSpinners />
    </Section>
  );
}
