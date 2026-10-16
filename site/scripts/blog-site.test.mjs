import test from "node:test";
import assert from "node:assert/strict";
import { readFile, mkdtemp, cp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generate } from "../src/blog/build.mjs";
import { filesBelow, indexBody, layout, locationConfig } from "../src/blog/site.mjs";

const meta = {
  slug: "agent-conversations",
  title: "Title & <markup>",
  summary: "Summary",
  date: "2026-10-16",
  language: "en",
};
const site = { title: "TMT Journal", description: "Notes" };
test("blog URLs derive from the site base and escape metadata", () => {
  for (const base of ["/", "/tmt/", "/custom/path/"]) {
    const location = locationConfig({ base });
    assert.equal(location.base, base + "blog/");
    const body = indexBody([{ meta }], site, location);
    assert.ok(body.includes(`href="${base}blog/posts/agent-conversations/"`));
    assert.match(body, /Title &amp; &lt;markup&gt;/);
    const html = layout({ site, location, title: "Blog", description: "Notes", body });
    assert.ok(html.includes(`href="${base}"`));
    assert.ok(html.includes(`href="https://pj-tmt.github.io${base}blog/"`));
  }
  for (const base of ["../", "//host/", "/a/../", "/missing-slash"])
    assert.throws(() => locationConfig({ base }));
});

test("generation renders articles; production removes demo inputs without touching approved sources", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-build-"));
  try {
    for (const path of ["src/blog", "src/posts", "scripts/fixtures/blog"])
      await cp(path, join(root, path), { recursive: true });
    const demo = await generate({ root, demo: true, base: "/nested/" });
    const html = await readFile(
      join(demo.output, "posts/interactive-article-example/index.html"),
      "utf8",
    );
    assert.match(html, /How many connections/);
    assert.match(html, /noindex,nofollow/);
    assert.doesNotMatch(demo.publicFiles["feed.xml"], /<item>/);
    await generate({ root, base: "/nested/" });
    const first = await filesBelow(join(root, "blog"));
    assert.equal(
      Object.keys(first).some((path) => path.includes("interactive-article-example")),
      false,
    );
    assert.doesNotMatch(first["index.html"].toString(), /Template preview/);
    await generate({ root, base: "/nested/" });
    assert.deepEqual(await filesBelow(join(root, "blog")), first);
    assert.equal((await filesBelow(join(root, "src/posts")))[".gitkeep"].length, 0);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
