import { busyFrames, browserUiClasses as c } from './static';

export { busyFrames };

/**
 * The one leaf busy mark: the braille cycle the Ops TUI shows, a 1em mono square in the current
 * colour that steps through the ten frames once per --tmt-ui-spin-duration. It is decorative; the
 * action or row that shows it names the state. Under reduced motion it holds the first frame.
 * Each frame is drawn by CSS from `data-frame`, so the mark adds no text to its host.
 */
export function BusyMark({ className }: { className?: string }) {
  return (
    <span className={className ? `${c.busyMark} ${className}` : c.busyMark} aria-hidden="true">
      <span className={c.busyFrames}>
        {busyFrames.map((frame) => (
          <span key={frame} data-frame={frame} />
        ))}
      </span>
    </span>
  );
}
