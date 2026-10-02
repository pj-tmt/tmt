// Fails when a chapter uses a JSX component it does not import (#999): MDX
// reports that only at runtime, after the page has shipped.
import { existsSync, readdirSync, readFileSync } from "node:fs";

const root = new URL("../src/", import.meta.url);
// English chapters and every translation directory under src/i18n/.
const dirs = [new URL("chapters/", root)];
const i18n = new URL("i18n/", root);
if (existsSync(i18n)) {
  for (const entry of readdirSync(i18n, { withFileTypes: true })) {
    if (entry.isDirectory()) dirs.push(new URL(`${entry.name}/`, i18n));
  }
}
const problems = [];
const pages = dirs.flatMap((dir) =>
  readdirSync(dir)
    .filter((name) => name.endsWith(".mdx"))
    .map((name) => ({ dir, name })),
);
for (const { dir, name: file } of pages) {
  const text = readFileSync(new URL(file, dir), "utf8");
  const imported = new Set();
  for (const [, names] of text.matchAll(/^import\s*\{([^}]*)\}\s*from/gm)) {
    for (const name of names.split(","))
      imported.add(
        name
          .trim()
          .split(/\s+as\s+/)
          .pop(),
      );
  }
  for (const [, name] of text.matchAll(/^import\s+([A-Z]\w*)\s+from/gm)) imported.add(name);
  // Ignore fenced code, inline code and template literals, where JSX-like text is literal.
  const prose = text.replace(/^```[\s\S]*?^```/gm, "").replace(/`[^`]*`/g, "");
  for (const [, name] of prose.matchAll(/<([A-Z][\w.]*)/g)) {
    const root = name.split(".")[0];
    if (!imported.has(root))
      problems.push(
        `${new URL(file, dir).pathname.split("/src/")[1]}: <${name}> is used but not imported`,
      );
  }
}
if (problems.length) {
  console.error([...new Set(problems)].join("\n"));
  process.exit(1);
}
