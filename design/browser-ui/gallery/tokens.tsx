import { useLayoutEffect, useRef, useState } from 'react';
import tokens from '../../tokens/tokens.json';
import { BrowserAction } from '../src/react.js';
import { Pair, Section, noop, type Theme } from './kit.js';
import { ThemeCell } from './theme-cell.js';

type Rendering = { light: string; dark: string };
const roles = (group: Record<string, unknown>) =>
  Object.entries(group).filter(([name]) => !name.startsWith('$')) as [string, Rendering][];

/** The value a swatch really paints, read back from the page, so a scope such as palette B shows its
 * own values (#5821); `fallback` is the token document's value until the swatch has painted. */
function LiveValue({ fallback, from }: { fallback: string; from: () => HTMLElement | null }) {
  const [value, setValue] = useState(fallback);
  useLayoutEffect(() => {
    const node = from();
    if (!node) return;
    const parts = getComputedStyle(node)
      .backgroundColor.match(/[\d.]+/g)
      ?.map(Number);
    if (!parts || parts.length < 3) return;
    const hex = parts
      .slice(0, 3)
      .map((part) => part.toString(16).padStart(2, '0'))
      .join('');
    const alpha =
      parts.length > 3 && parts[3]! < 1
        ? Math.round(parts[3]! * 255)
            .toString(16)
            .padStart(2, '0')
        : '';
    setValue(`#${hex}${alpha}`);
  });
  return <span className="gallery-value">{value}</span>;
}

function Swatch({ name, prefix, fallback }: { name: string; prefix: string; fallback: string }) {
  const swatch = useRef<HTMLSpanElement>(null);
  return (
    <li>
      <span
        ref={swatch}
        className="gallery-swatch"
        style={{ background: `var(--tmt-ui-${prefix}-${name})` }}
      />
      <code>{name}</code>
      <LiveValue fallback={fallback} from={() => swatch.current} />
    </li>
  );
}

function Swatches({
  group,
  prefix,
  theme,
}: {
  group: Record<string, unknown>;
  prefix: string;
  theme: Theme;
}) {
  return (
    <ul className="gallery-swatches">
      {roles(group).map(([name, value]) => (
        <Swatch key={name} name={name} prefix={prefix} fallback={value[theme].toLowerCase()} />
      ))}
    </ul>
  );
}

/** The primitive tier's colours (`tokens.browser.ref`), the raw values palette B's roles read. */
function RefSwatches() {
  return (
    <ul className="gallery-swatches">
      {Object.entries(tokens.browser.ref)
        .filter(([name, value]) => !name.startsWith('$') && String(value).startsWith('#'))
        .map(([name, value]) => (
          <Swatch key={name} name={name} prefix="ref" fallback={String(value)} />
        ))}
    </ul>
  );
}

const typeScale = [
  ['Header wordmark', 'header-wordmark', 'display'],
  ['Header title', 'header-title', 'display'],
  ['Header action', 'header-action', 'body'],
  ['Header mark', 'header-mark', 'mono'],
] as const;

function Type() {
  return (
    <div className="gallery-stack">
      {Object.entries(tokens.font).map(([name, font]) => (
        <p key={name} className="gallery-type" style={{ fontFamily: `var(--tmt-ui-font-${name})` }}>
          <code>font.{name}</code> <span className="gallery-value">{font.use}</span>
          <span className="gallery-type-sample">Talk, measure, tune — 0123456789</span>
        </p>
      ))}
      {typeScale.map(([label, key, family]) => (
        <p
          key={key}
          className="gallery-type"
          style={{
            font: `var(--tmt-ui-${key}-weight) var(--tmt-ui-${key}-size) / var(--tmt-ui-${key}-line-height) var(--tmt-ui-font-${family})`,
          }}
        >
          <code>header.{key.slice('header-'.length)}</code>{' '}
          <span className="gallery-value">
            {tokens.header[`${key.slice('header-'.length)}-size` as 'title-size']} ·{' '}
            {tokens.header[`${key.slice('header-'.length)}-weight` as 'title-weight']}
          </span>
          <span className="gallery-type-sample">{label}</span>
        </p>
      ))}
      <p className="gallery-type">
        <code>eyebrow</code>{' '}
        <span className="gallery-value">{tokens.browser.metric['eyebrow-size']}</span>
        <span
          className="gallery-type-sample"
          style={{
            font: 'var(--tmt-ui-eyebrow-weight) var(--tmt-ui-eyebrow-size) var(--tmt-ui-font-mono)',
            letterSpacing: 'var(--tmt-ui-eyebrow-tracking)',
            textTransform: 'uppercase',
          }}
        >
          Needs you
        </span>
      </p>
      <p className="gallery-type">
        <code>caption</code>{' '}
        <span className="gallery-value">{tokens.browser.metric['caption-size']}</span>
        <span
          className="gallery-type-sample"
          style={{
            font: 'var(--tmt-ui-caption-size) / var(--tmt-ui-caption-line-height) var(--tmt-ui-font-body)',
          }}
        >
          Updated 2 minutes ago
        </span>
      </p>
    </div>
  );
}

const length = /^\d+(\.\d+)?px$/;

/** The motion tokens at work: one replay drives a fade, a pop and a slide on the real durations. */
function MotionDemo() {
  const [shown, setShown] = useState(true);
  const move = (property: string, duration: string, from: string, to: string) => ({
    transition: `${property} var(--tmt-ui-motion-${duration}) var(--tmt-ui-motion-ease-${shown ? 'out' : 'in'})`,
    ...(shown ? { [property]: to } : { [property]: from }),
  });
  const box = {
    width: 72,
    height: 48,
    border: '1px solid var(--tmt-ui-surface-rule)',
    borderRadius: 'var(--tmt-ui-radius-l)',
    background: 'var(--tmt-ui-surface-sheet)',
  };
  return (
    <div className="gallery-stack">
      <div className="gallery-row">
        <BrowserAction
          type="button"
          variant="primary"
          label={shown ? 'Leave' : 'Enter'}
          onActivate={() => setShown((value) => !value)}
        />
        <span className="gallery-note">
          fast {tokens.browser.motion.fast} · base {tokens.browser.motion.base} · none under reduced
          motion
        </span>
      </div>
      <div className="gallery-row">
        <figure style={{ margin: 0 }}>
          <div style={{ ...box, ...move('opacity', 'base', '0', '1') }} />
          <figcaption className="gallery-cell-label">fade · base</figcaption>
        </figure>
        <figure style={{ margin: 0 }}>
          <div
            style={{
              ...box,
              transition: `opacity var(--tmt-ui-motion-base) var(--tmt-ui-motion-ease-${shown ? 'out' : 'in'}), scale var(--tmt-ui-motion-base) var(--tmt-ui-motion-ease-${shown ? 'out' : 'in'})`,
              opacity: shown ? 1 : 0,
              scale: shown ? '1' : 'var(--tmt-ui-motion-scale)',
            }}
          />
          <figcaption className="gallery-cell-label">pop · base (dialog)</figcaption>
        </figure>
        <figure style={{ margin: 0 }}>
          <div style={{ overflow: 'hidden', width: 120, height: 48 }}>
            <div
              style={{
                ...box,
                width: 120,
                transition: `translate var(--tmt-ui-motion-base) var(--tmt-ui-motion-ease-${shown ? 'out' : 'in'})`,
                translate: shown ? '0 0' : '100% 0',
              }}
            />
          </div>
          <figcaption className="gallery-cell-label">slide · base (drawer)</figcaption>
        </figure>
        <figure style={{ margin: 0 }}>
          <div
            style={{
              ...box,
              transition: `opacity var(--tmt-ui-motion-fast) var(--tmt-ui-motion-ease-${shown ? 'out' : 'in'}), scale var(--tmt-ui-motion-fast) var(--tmt-ui-motion-ease-${shown ? 'out' : 'in'})`,
              opacity: shown ? 1 : 0,
              scale: shown ? '1' : 'var(--tmt-ui-motion-scale)',
            }}
          />
          <figcaption className="gallery-cell-label">pop · fast (menu, tooltip)</figcaption>
        </figure>
      </div>
      <p className="gallery-note">
        Hover and press on actions use the fast duration:{' '}
        <BrowserAction type="button" variant="text" label="Press me" onActivate={noop} />
      </p>
    </div>
  );
}

export function TokensSection() {
  return (
    <Section id="tokens">
      <Pair
        title="Primitive tier (tokens.browser.ref)"
        note="The raw values palette B's roles read; the labels show what each swatch paints."
      >
        {() => <RefSwatches />}
      </Pair>
      <Pair title="Colour roles (tokens.browser.color)">
        {(theme) => <Swatches group={tokens.browser.color} prefix="color" theme={theme} />}
      </Pair>
      <Pair title="Surfaces (tokens.browser.surface)">
        {(theme) => <Swatches group={tokens.browser.surface} prefix="surface" theme={theme} />}
      </Pair>
      <Pair title="Type (tokens.font, tokens.header)">{() => <Type />}</Pair>
      <Pair
        title="Motion (tokens.browser.motion)"
        note="Overlays enter and leave with these; replay shows each on the real tokens."
      >
        {() => <MotionDemo />}
      </Pair>
      <div className="gallery-pair">
        <h3>Spacing and sizes (tokens.browser.metric, tokens.header)</h3>
        <p className="gallery-note">
          Metrics do not change with the theme. Bars use the generated custom property.
        </p>
        <div className="gallery-scroll">
          <table className="gallery-metrics">
            <tbody>
              {[
                ...Object.entries(tokens.browser.metric).map(
                  ([name, value]) => [name, value] as const,
                ),
                ...Object.entries(tokens.header).map(
                  ([name, value]) => [`header-${name}`, value] as const,
                ),
              ].map(([name, value]) => (
                <tr key={name}>
                  <td>
                    <code>--tmt-ui-{name}</code>
                  </td>
                  <td className="gallery-value">{value}</td>
                  <td>
                    {length.test(value) && (
                      <span className="gallery-bar" style={{ width: `var(--tmt-ui-${name})` }} />
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
      <Pair
        title="Radius, focus and indicator"
        note="Surfaces take the radius scale (chip, control, card, pill). Keyboard focus is the focus role: a recoloured edge on a bordered control, an inset outline on a borderless one; a destructive control uses its own red. The indicator is a separate straight bar, never a one-sided border on a rounded box."
      >
        {() => (
          <div className="gallery-row">
            {(['radius-s', 'radius', 'radius-l', 'radius-pill'] as const).map((name) => (
              <span
                key={name}
                className="gallery-radius"
                style={{ borderRadius: `var(--tmt-ui-${name})` }}
              >
                {name} <code>{tokens.browser.metric[name]}</code>
              </span>
            ))}
            <span className="gallery-focus">
              focus <code>{tokens.browser.metric['focus-width']}</code>
            </span>
            <span className="gallery-focus gallery-focus-destructive">
              focus destructive <code>{tokens.browser.metric['focus-width']}</code>
            </span>
            <span className="gallery-indicator">
              <span className="gallery-indicator-bar" aria-hidden="true" />
              indicator <code>{tokens.browser.metric['indicator-width']}</code>
            </span>
          </div>
        )}
      </Pair>
      <ThemeCell />
    </Section>
  );
}
