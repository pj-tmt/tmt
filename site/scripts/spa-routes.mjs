// Publish only Home. Historical chapter files are redirect stubs, never app pages.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const languages = [
  { prefix: "", htmlLang: "en" },
  { prefix: "ja", htmlLang: "ja" },
  { prefix: "zh-hant", htmlLang: "zh-Hant" },
  { prefix: "zh-hans", htmlLang: "zh-Hans" },
];
const escape = (value) =>
  value.replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;");

export function writeRoutes(site, { base = "/tmt/", origin = "", hashed = false } = {}) {
  const dist = join(site, "dist");
  const template = readFileSync(join(dist, "index.html"), "utf8");
  const registry = readFileSync(join(site, "src/chapters/index.ts"), "utf8");
  const paths = [...new Set([...registry.matchAll(/path: "(\/[^"]*)"/g)].map((match) => match[1]))];
  const absolute = base.startsWith("/") && !hashed;
  const homeHref = (prefix) => `${origin.replace(/\/+$/, "")}${base}${prefix ? `${prefix}/` : ""}`;
  const write = (directory, html) => {
    const target = join(dist, directory, "index.html");
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, html);
  };
  for (const language of languages) {
    let html = template.replace(/<html lang="[^"]*"/, `<html lang="${language.htmlLang}"`);
    if (absolute) {
      const links = languages.map(
        (other) =>
          `<link rel="alternate" hreflang="${other.htmlLang}" href="${escape(homeHref(other.prefix))}" />`,
      );
      links.push(`<link rel="alternate" hreflang="x-default" href="${escape(homeHref(""))}" />`);
      links.push(`<link rel="canonical" href="${escape(homeHref(language.prefix))}" />`);
      html = html.replace("</head>", `${links.join("\n")}\n</head>`);
    }
    write(language.prefix, html);
  }
  let redirects = 0;
  for (const language of [...languages, { prefix: "zh", htmlLang: "zh-Hant" }]) {
    for (const path of paths) {
      if (path === "/" && language.prefix !== "zh") continue;
      const directory = [language.prefix, path.replace(/^\/+/, "")].filter(Boolean).join("/");
      const prefix = language.prefix === "zh" ? "zh-hant" : language.prefix;
      // Relative previews need a path back to their own deployment root.
      const target = absolute
        ? homeHref(prefix)
        : `${"../".repeat(directory.split("/").length)}${hashed ? `#/${prefix}` : prefix ? `${prefix}/` : ""}`;
      const url = escape(target);
      write(
        directory,
        `<!doctype html><html lang="${language.htmlLang}"><head><meta charset="utf-8"><meta name="robots" content="noindex"><link rel="canonical" href="${url}"><meta http-equiv="refresh" content="0;url=${url}"><title>tmt Handbook</title></head><body><a href="${url}">tmt</a></body></html>`,
      );
      redirects += 1;
    }
  }
  // Unknown deep links load only the Home app, which normalizes the locale URL.
  writeFileSync(
    join(dist, "404.html"),
    template.replace("</head>", '<meta name="robots" content="noindex"></head>'),
  );
  return { homes: languages.length, redirects };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = writeRoutes(dirname(dirname(fileURLToPath(import.meta.url))), {
    base: process.env.SITE_BASE ?? "/tmt/",
    origin: process.env.SITE_ORIGIN ?? "",
    hashed: process.env.VITE_SITE_HISTORY === "hash",
  });
  console.log(
    `spa-routes: ${result.homes} Home routes, ${result.redirects} legacy redirects and 404.html`,
  );
}
