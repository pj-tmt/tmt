import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import tokens from '../../tokens/tokens.json';
import { BrandPage, ElevationPage, MotionPage } from './motion.js';
import { usePreview } from './preview.js';

const names = (group: Record<string, unknown>) =>
  Object.keys(group).filter((name) => !name.startsWith('$'));

/** A custom property's computed value on the document, re-read when the theme or radius changes. */
function useComputed(property: string, measure?: (node: HTMLElement) => string) {
  const { theme, radius } = usePreview();
  const ref = useRef<HTMLSpanElement>(null);
  const [value, setValue] = useState('');
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    setValue(
      measure ? measure(node) : getComputedStyle(node).getPropertyValue(property).trim() || '—',
    );
  }, [property, measure, theme, radius]);
  return { ref, value };
}
const hex = (node: HTMLElement) => {
  const parts =
    getComputedStyle(node)
      .backgroundColor.match(/[\d.]+/g)
      ?.map(Number) ?? [];
  const rgb = parts
    .slice(0, 3)
    .map((part) => part.toString(16).padStart(2, '0'))
    .join('');
  const alpha =
    parts.length > 3 && parts[3]! < 1
      ? Math.round(parts[3]! * 255)
          .toString(16)
          .padStart(2, '0')
      : '';
  return `#${rgb}${alpha}`;
};

function Swatch({ variable, name }: { variable: string; name: string }) {
  const { ref, value } = useComputed(variable, hex);
  return (
    <li className="docs-swatch">
      <span ref={ref} className="docs-swatch-chip" style={{ background: `var(${variable})` }} />
      <code>{name}</code>
      <span className="docs-value">{value}</span>
    </li>
  );
}

function Swatches({ prefix, list }: { prefix: string; list: string[] }) {
  return (
    <ul className="docs-swatches">
      {list.map((name) => (
        <Swatch key={name} variable={`--tmt-ui-${prefix}-${name}`} name={name} />
      ))}
    </ul>
  );
}

function Measured({
  variable,
  children,
}: {
  variable: string;
  children: (value: string) => ReactNode;
}) {
  const { ref, value } = useComputed(variable);
  return <span ref={ref}>{children(value)}</span>;
}

const refColours = Object.entries(tokens.browser.ref)
  .filter(([name, value]) => !name.startsWith('$') && String(value).startsWith('#'))
  .map(([name]) => name);
const statusRoles = ['working', 'waiting', 'review', 'blocked'];
const steps = Array.from({ length: 9 }, (_, index) => `size-${index + 1}`);
const spaces = Array.from({ length: 6 }, (_, index) => `space-${index + 1}`);

export const tokenPages: { slug: string; title: string; lead: string; render(): ReactNode }[] = [
  {
    slug: 'colour',
    title: 'Colour',
    lead: 'Every swatch reads its value from the page, so it shows what the current theme really paints.',
    render: () => (
      <>
        <h2>Semantic roles</h2>
        <p>What components read. Under palette B each role points at one ref.</p>
        <Swatches
          prefix="color"
          list={names(tokens.browser.color).filter((name) => !/^(series|seat)-/.test(name))}
        />
        <h2>Surfaces</h2>
        <Swatches prefix="surface" list={names(tokens.browser.surface)} />
        <h2>Status</h2>
        <p>Muted hues, always with a mark and a word. Blocked is never the accent.</p>
        <ul className="docs-status">
          {statusRoles.map((role) => (
            <li key={role} style={{ color: `var(--tmt-ui-color-${role})` }}>
              {{ working: '●', waiting: '◆', review: '◐', blocked: '✕' }[role]}{' '}
              {role[0]!.toUpperCase() + role.slice(1)}
            </li>
          ))}
        </ul>
        <h2>Ref tier</h2>
        <p>
          The primitive values, <code>--tmt-ui-ref-*</code>. Only roles read them.
        </p>
        <Swatches prefix="ref" list={refColours} />
        <h2>Data colours</h2>
        <p>Chart series and member seats keep their own hues.</p>
        <Swatches
          prefix="color"
          list={names(tokens.browser.color).filter((name) => /^(series|seat)-/.test(name))}
        />
      </>
    ),
  },
  {
    slug: 'type',
    title: 'Type',
    lead: 'One ramp of nine steps. Inter for text and headings, JetBrains Mono for commands and code.',
    render: () => (
      <>
        <ul className="docs-ramp">
          {steps.map((step) => (
            <li key={step}>
              <Measured variable={`--tmt-ui-ref-${step}`}>
                {(value) => (
                  <>
                    <span className="docs-ramp-meta">
                      <code>{step}</code> {value}
                    </span>
                    <span style={{ fontSize: `var(--tmt-ui-ref-${step})` }}>
                      Talk, measure, tune
                    </span>
                  </>
                )}
              </Measured>
            </li>
          ))}
        </ul>
        <h2>Families</h2>
        <p style={{ fontFamily: 'var(--tmt-ui-font-body)' }}>Inter — body and headings.</p>
        <p style={{ fontFamily: 'var(--tmt-ui-font-mono)' }}>
          JetBrains Mono — tmt colab page check
        </p>
      </>
    ),
  },
  {
    slug: 'spacing',
    title: 'Spacing',
    lead: 'Six steps on a 4px grid. Containers space their children with a gap, never with margins alone.',
    render: () => (
      <ul className="docs-scale">
        {spaces.map((space) => (
          <li key={space}>
            <Measured variable={`--tmt-ui-ref-${space}`}>
              {(value) => (
                <>
                  <code>{space}</code>
                  <span className="docs-scale-value">{value}</span>
                  <span className="docs-bar" style={{ width: `var(--tmt-ui-ref-${space})` }} />
                </>
              )}
            </Measured>
          </li>
        ))}
      </ul>
    ),
  },
  {
    slug: 'radius',
    title: 'Radius',
    lead: 'One knob. Controls, fields and tags take it; cards and surfaces take one step more. Try the switch in the top bar.',
    render: () => (
      <ul className="docs-radii">
        {[
          ['radius-s', 'Small controls'],
          ['radius', 'Controls, fields, tags'],
          ['radius-badge', 'Counts'],
          ['radius-l', 'Cards and surfaces'],
          ['radius-pill', 'Avatars and status dots'],
        ].map(([role, use]) => (
          <li key={role}>
            <span className="docs-radius-box" style={{ borderRadius: `var(--tmt-ui-${role})` }} />
            <code>{role}</code>
            <Measured variable={`--tmt-ui-${role}`}>{(value) => <span>{value}</span>}</Measured>
            <span className="docs-muted">{use}</span>
          </li>
        ))}
      </ul>
    ),
  },
  {
    slug: 'motion',
    title: 'Motion',
    lead: 'Two durations and two easings, from the motion tokens only. Under reduced motion every duration is 0ms.',
    render: () => (
      <>
        <MotionPage />
        <h2>Tokens</h2>
        <ul className="docs-motion">
          {names(tokens.browser.motion).map((name) => (
            <li key={name}>
              <code>{name}</code>
              <Measured variable={`--tmt-ui-motion-${name}`}>
                {(value) => <span>{value}</span>}
              </Measured>
            </li>
          ))}
        </ul>
      </>
    ),
  },
  {
    slug: 'elevation',
    title: 'Elevation',
    lead: 'Surface levels on one elevation scale, from the page to a modal.',
    render: () => <ElevationPage />,
  },
  {
    slug: 'brand',
    title: 'Brand',
    lead: 'The mark under palette B, at the sizes the product draws it.',
    render: () => <BrandPage />,
  },
];
