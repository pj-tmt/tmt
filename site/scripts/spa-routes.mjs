// GitHub Pages serves files, not routes. Give every handbook page its own
// index.html, so a deep link or a reload returns 200, and a 404.html that loads
// the same app for anything else.
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const site = dirname(dirname(fileURLToPath(import.meta.url)));
const dist = join(site, "dist");
const index = join(dist, "index.html");
const registry = readFileSync(join(site, "src/chapters/index.ts"), "utf8");
const paths = [
  ...new Set([...registry.matchAll(/path: "(\/[^"]+)"/g)].map((match) => match[1])),
].filter((path) => path !== "/");
for (const path of paths) {
  const target = join(dist, path, "index.html");
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(index, target);
}
copyFileSync(index, join(dist, "404.html"));
console.log(`spa-routes: ${paths.length} page routes and 404.html`);
