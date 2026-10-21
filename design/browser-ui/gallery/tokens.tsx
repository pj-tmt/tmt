import tokens from '../../tokens/tokens.json';
import { Pair, Section, type Theme } from './kit.js';
import { ThemeCell } from './theme-cell.js';

type Rendering = { light: string; dark: string };
const roles = (group: Record<string, unknown>) =>
  Object.entries(group).filter(([name]) => !name.startsWith('$')) as [string, Rendering][];

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
        <li key={name}>
          <span
            className="gallery-swatch"
            style={{ background: `var(--tmt-ui-${prefix}-${name})` }}
          />
          <code>{name}</code>
          <span className="gallery-value">{value[theme].toLowerCase()}</span>
        </li>
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

export function TokensSection() {
  return (
    <Section id="tokens">
      <Pair title="Colour roles (tokens.browser.color)">
        {(theme) => <Swatches group={tokens.browser.color} prefix="color" theme={theme} />}
      </Pair>
      <Pair title="Surfaces (tokens.browser.surface)">
        {(theme) => <Swatches group={tokens.browser.surface} prefix="surface" theme={theme} />}
      </Pair>
      <Pair title="Type (tokens.font, tokens.header)">{() => <Type />}</Pair>
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
        title="Radius and focus"
        note="Every shared surface is square; keyboard focus is a solid outline of the focus role."
      >
        {() => (
          <div className="gallery-row">
            <span className="gallery-radius">
              radius <code>{tokens.browser.metric.radius}</code>
            </span>
            <span className="gallery-focus">
              focus <code>{tokens.browser.metric['focus-width']}</code>
            </span>
            <span className="gallery-indicator">
              indicator <code>{tokens.browser.metric['indicator-width']}</code>
            </span>
          </div>
        )}
      </Pair>
      <ThemeCell />
    </Section>
  );
}
