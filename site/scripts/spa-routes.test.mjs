import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { writeRoutes } from "./spa-routes.mjs";

function fixture(t) {
  const site = mkdtempSync(join(tmpdir(), "tmt-home-routes-"));
  t.after(() => rmSync(site, { recursive: true, force: true }));
  mkdirSync(join(site, "dist"));
  mkdirSync(join(site, "src/chapters"), { recursive: true });
  writeFileSync(
    join(site, "dist/index.html"),
    '<html lang="en"><head></head><body><script src="/tmt/assets/home.js"></script></body></html>',
  );
  writeFileSync(
    join(site, "src/chapters/index.ts"),
    'path: "/", path: "/working", path: "/extensions/squad"',
  );
  return { site, read: (path) => readFileSync(join(site, "dist", path), "utf8") };
}

test("only locale Home files contain the app; every old chapter is a noindex redirect", (t) => {
  const { site, read } = fixture(t);
  assert.deepEqual(writeRoutes(site), { homes: 4, redirects: 11 });
  for (const prefix of ["", "ja/", "zh-hant/", "zh-hans/"]) {
    const home = read(`${prefix}index.html`);
    assert.match(home, /<script/);
    assert.match(home, /hreflang="x-default" href="\/tmt\/"/);
    assert.doesNotMatch(home, /href="[^"]*(?:working|squad)/);
    for (const path of ["working", "extensions/squad"]) {
      const old = read(`${prefix}${path}/index.html`);
      assert.doesNotMatch(old, /<script|hreflang/);
      assert.ok(old.includes(`content="0;url=/tmt/${prefix}"`));
      assert.match(old, /content="noindex"/);
    }
  }
  assert.match(read("zh/working/index.html"), /0;url=\/tmt\/zh-hant\//);
  assert.match(read("404.html"), /noindex/);
});

test("custom origin and relative previews redirect within their deployment", (t) => {
  const a = fixture(t);
  writeRoutes(a.site, { base: "/", origin: "https://example.org/" });
  assert.match(a.read("ja/working/index.html"), /0;url=https:\/\/example.org\/ja\//);
  const b = fixture(t);
  writeRoutes(b.site, { base: "./", hashed: true });
  assert.match(b.read("ja/extensions/squad/index.html"), /0;url=\.\.\/\.\.\/\.\.\/#\/ja/);
  assert.doesNotMatch(b.read("index.html"), /hreflang/);
});
