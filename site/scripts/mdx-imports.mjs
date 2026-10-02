// Fails when a chapter uses a JSX component it does not import (#999): MDX
// reports that only at runtime, after the page has shipped.
import { readdirSync, readFileSync } from "node:fs";

const dir = new URL("../src/chapters/", import.meta.url);
const problems = [];
for (const file of readdirSync(dir).filter((name) => name.endsWith(".mdx"))) {
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
    if (!imported.has(root)) problems.push(`${file}: <${name}> is used but not imported`);
  }
}
if (problems.length) {
  console.error([...new Set(problems)].join("\n"));
  process.exit(1);
}
