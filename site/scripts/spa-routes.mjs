// GitHub Pages serves files, not routes. Give every handbook page its own
// index.html in every language (/, /ja/, /zh-hant/), so a deep link or a reload
// returns 200, and a 404.html that loads the same app for anything else.
// Each file also carries what a crawler reads without running the app: its
// <html lang> and the hreflang alternates for the same page in every language.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const site = dirname(dirname(fileURLToPath(import.meta.url)));
const dist = join(site, "dist");
const index = join(dist, "index.html");
const template = readFileSync(index, "utf8");
const registry = readFileSync(join(site, "src/chapters/index.ts"), "utf8");
const pagePaths = [
  ...new Set([...registry.matchAll(/path: "(\/[^"]*)"/g)].map((match) => match[1])),
];

// Mirrors src/lang/languages.ts: URL prefix and the <html lang> / hreflang value.
const languages = [
  { prefix: "", htmlLang: "en" },
  { prefix: "ja", htmlLang: "ja" },
  { prefix: "zh-hant", htmlLang: "zh-Hant" },
];

// The same base as vite.config.ts. Alternates need addresses a crawler can use,
// so a relative base (a preview at an unknown path) or hash routing gets none.
// SITE_ORIGIN, when set, makes them absolute (https://example.org).
const base = process.env.SITE_BASE ?? "/tmt/";
const alternates = base.startsWith("/") && process.env.VITE_SITE_HISTORY !== "hash";
const origin = (process.env.SITE_ORIGIN ?? "").replace(/\/+$/, "");

const directory = (prefix, pagePath) =>
  [prefix, pagePath.replace(/^\/+/, "")].filter(Boolean).join("/");
const href = (prefix, pagePath) => {
  const dir = directory(prefix, pagePath);
  return `${origin}${base}${dir ? `${dir}/` : ""}`;
};

function render(language, pagePath) {
  let html = template.replace(/<html lang="[^"]*"/, `<html lang="${language.htmlLang}"`);
  if (!alternates) return html;
  const links = [
    ...languages.map(
      (other) =>
        `<link rel="alternate" hreflang="${other.htmlLang}" href="${href(other.prefix, pagePath)}" />`,
    ),
    `<link rel="alternate" hreflang="x-default" href="${href("", pagePath)}" />`,
  ];
  return html.replace(
    /([ \t]*)<\/head>/,
    (_, indent) => `${indent}${links.join(`\n${indent}`)}\n${indent}</head>`,
  );
}

let files = 0;
for (const language of languages) {
  for (const pagePath of pagePaths) {
    const dir = directory(language.prefix, pagePath);
    const target = join(dist, dir, "index.html");
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, render(language, pagePath));
    files += 1;
  }
}
// Keep published /zh links working. The router replaces the path while retaining
// query and hash; the canonical identifies the new route to crawlers.
for (const pagePath of pagePaths) {
  const target = join(dist, directory("zh", pagePath), "index.html");
  mkdirSync(dirname(target), { recursive: true });
  const html = render(
    languages.find((language) => language.prefix === "zh-hant"),
    pagePath,
  );
  writeFileSync(
    target,
    html.replace("</head>", `<link rel="canonical" href="${href("zh-hant", pagePath)}" /></head>`),
  );
}
writeFileSync(join(dist, "404.html"), template);
console.log(`spa-routes: ${files} page routes (${languages.length} languages) and 404.html`);
