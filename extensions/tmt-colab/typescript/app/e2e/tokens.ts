import { readFileSync } from 'node:fs';

/** The design tokens the specs read their expected colours and metrics from. */
export const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
) as {
  header: Record<string, string>;
  browser: {
    color: Record<string, Record<'light' | 'dark', string>>;
    surface: Record<string, Record<'light' | 'dark', string>>;
    metric: Record<string, string>;
    ui: Record<string, string>;
  };
};

/** `#rrggbb` as the browser reports a computed colour. */
export function rgb(hex: string): string {
  return `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(', ')})`;
}

/** The shared bar height at a viewport width, including its token breakpoint. */
export function headerHeight(width: number): number {
  return Number.parseFloat(
    tokens.header[
      width <= Number.parseFloat(tokens.header['compact-max-width']!) ? 'compact-height' : 'height'
    ]!,
  );
}
