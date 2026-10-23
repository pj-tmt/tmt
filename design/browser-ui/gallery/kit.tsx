import { useLayoutEffect, useRef, type ReactNode } from 'react';
import { pinState, type ForcedState } from './force-states.js';
import type { SectionId } from './covered.js';
import { sections } from './covered.js';

export type Theme = 'light' | 'dark';
export const themes: readonly Theme[] = ['light', 'dark'];

export const noop = () => {};

export function Section({ id, children }: { id: SectionId; children: ReactNode }) {
  const title = sections.find(([key]) => key === id)![1];
  return (
    <section className="gallery-section" id={id} aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`}>{title}</h2>
      {children}
    </section>
  );
}

/** One demo in light and dark, side by side; each column scopes the leaf's real theme roles. */
export function Pair({
  title,
  note,
  stacked = false,
  children,
}: {
  title: string;
  note?: ReactNode;
  /** The themes stack, so the demo keeps the page's full width. */
  stacked?: boolean;
  children: (theme: Theme) => ReactNode;
}) {
  return (
    <div className="gallery-pair">
      <h3>{title}</h3>
      {note && <p className="gallery-note">{note}</p>}
      <div className="gallery-columns" data-stacked={stacked || undefined}>
        {themes.map((theme) => (
          <div key={theme} className="gallery-column" data-theme={theme}>
            <span className="gallery-theme-label">{theme}</span>
            {children(theme)}
          </div>
        ))}
      </div>
    </div>
  );
}

/** A labelled state cell; `pin` holds hover or focus on its first control through real CSS. */
export function Cell({
  label,
  pin,
  children,
}: {
  label: string;
  pin?: ForcedState;
  children: ReactNode;
}) {
  const cell = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => (pin ? pinState(cell.current!, pin) : undefined), [pin]);
  return (
    <div className="gallery-cell">
      <span className="gallery-cell-label">{label}</span>
      <div ref={cell} className="gallery-cell-body">
        {children}
      </div>
    </div>
  );
}

export function Cells({ children }: { children: ReactNode }) {
  return <div className="gallery-cells">{children}</div>;
}

/** A real page of this gallery in an iframe, for demos that need a viewport or the top layer. */
export function Frame({
  frame,
  theme,
  width,
  height,
  params = {},
}: {
  frame: string;
  theme: Theme;
  width?: number;
  height: number;
  params?: Record<string, string>;
}) {
  const query = new URLSearchParams({ frame, theme, ...params });
  return (
    <iframe
      className="gallery-frame"
      loading="lazy"
      title={`${frame} ${theme}`}
      src={`?${query}`}
      style={{ width: width ?? '100%', height }}
    />
  );
}

/** A frame demo in light and dark, side by side. */
export function FramePair({
  title,
  note,
  frame,
  width,
  height,
  params,
}: {
  title: string;
  note?: ReactNode;
  frame: string;
  width?: number;
  height: number;
  params?: Record<string, string>;
}) {
  return (
    <div className="gallery-pair">
      <h3>{title}</h3>
      {note && <p className="gallery-note">{note}</p>}
      {/* A frame wider than half the page stacks its themes so its viewport keeps its width. */}
      <div className="gallery-columns" data-stacked={(width ?? 0) > 600 || undefined}>
        {themes.map((theme) => (
          <div key={theme} className="gallery-column gallery-frame-column" data-theme={theme}>
            <span className="gallery-theme-label">{theme}</span>
            <Frame frame={frame} theme={theme} width={width} height={height} params={params} />
          </div>
        ))}
      </div>
    </div>
  );
}
