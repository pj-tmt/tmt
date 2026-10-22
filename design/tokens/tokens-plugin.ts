import tokens from "./tokens.json" with { type: "json" };

type Rendering = { dark: string; light: string };

// Serves tokens.json as CSS custom properties, so the stylesheet and the
// design page read one source. `--c-*` follows the theme; `--t-*` is always the
// dark value, for terminal mockups.
export function designTokens(): {
  name: string;
  resolveId: (source: string) => string | undefined;
  load: (path: string) => string | undefined;
} {
  const id = "virtual:tokens.css";
  const resolved = "/__tmt-tokens.css";
  const entries = [
    ...Object.entries(tokens.color as Record<string, Rendering>),
    ...Object.entries(tokens.surface as Record<string, Rendering>),
  ];
  const vars = (mode: "dark" | "light") =>
    entries.map(([name, value]) => `--c-${name}:${value[mode]};`).join("");
  const terminal = entries.map(([name, value]) => `--t-${name}:${value.dark};`).join("");
  const fonts = Object.entries(tokens.font)
    .map(([name, value]) => `--f-${name}:${value.stack};`)
    .join("");
  const header = Object.entries(tokens.header)
    .map(([name, value]) => `--colab-header-${name}:${value};`)
    .join("");
  // The legacy roles listed in `unified` take their browser values: one palette everywhere.
  const lookup = (path: string, mode: "dark" | "light"): string => {
    const [where, fixed] = path.split("@");
    const value = where.split(".").reduce((node: any, key) => node[key], tokens) as Rendering;
    return value[(fixed as "dark" | undefined) ?? mode];
  };
  const unified = (mode: "dark" | "light") =>
    Object.entries(tokens.unified as Record<string, string>)
      .filter(([name]) => !name.startsWith("$"))
      .map(([name, path]) => `--c-${name}:${lookup(path, mode)};`)
      .join("");
  // The mark-only logo roles, under the same names the browser leaf's static CSS uses.
  const brand = (mode: "dark" | "light") =>
    Object.entries(tokens.brand.mark as Record<string, Rendering | string>)
      .filter(([name]) => !name.startsWith("$"))
      .map(([name, value]) => `--tmt-ui-brand-mark-${name}:${(value as Rendering)[mode]};`)
      .join("");
  const css =
    `:root{color-scheme:light;${vars("light")}${unified("light")}${brand("light")}${terminal}${fonts}${header}}` +
    `@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){color-scheme:dark;${vars("dark")}${unified("dark")}${brand("dark")}}}` +
    `:root[data-theme="dark"]{color-scheme:dark;${vars("dark")}${unified("dark")}${brand("dark")}}` +
    `@media (max-width:${tokens.header["compact-max-width"]}){:root{--colab-header-height:${tokens.header["compact-height"]};}}`;
  return {
    name: "tmt-design-tokens",
    resolveId: (source: string) => (source === id ? resolved : undefined),
    load: (path: string) => (path === resolved ? css : undefined),
  };
}
