import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

test("the site tab icon is exactly the shared light/dark SVG", () => {
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  const link = html.match(/<link\b[^>]*\brel="icon"[^>]*>/g);
  assert.equal(link?.length, 1);
  const href = link[0].match(/\bhref="([^"]+)"/)[1];
  assert.ok(href.startsWith("data:image/svg+xml,"));
  assert.deepEqual(
    Buffer.from(decodeURIComponent(href.slice("data:image/svg+xml,".length))),
    readFileSync(new URL("../../design/browser-ui/generated/favicon.svg", import.meta.url)),
  );
});
