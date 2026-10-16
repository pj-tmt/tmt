import test from "node:test";
import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "vite-plus";
import { blogBuild } from "../src/blog/build.mjs";
import { verifyDist } from "../src/blog/verify-dist.mjs";

test("Vite bundles complete article HTML and external modules at project and root bases", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-vite-"));
  try {
    for (const path of ["src/blog", "src/posts", "scripts/fixtures/blog"])
      await cp(path, join(root, path), { recursive: true });
    await writeFile(
      join(root, "index.html"),
      "<!doctype html><html><body><h1>Site home</h1></body></html>",
    );
    const bundle = async (base) => {
      const blog = await blogBuild({ root, base, demo: true, development: false });
      return build({
        configFile: false,
        root,
        base,
        logLevel: "silent",
        plugins: [blog.plugin],
        build: { rolldownOptions: { input: [join(root, "index.html"), ...blog.entries] } },
      });
    };
    for (const base of ["/tmt/", "/"]) {
      await bundle(base);
      const dist = join(root, "dist");
      await verifyDist(dist, { demo: true });
      await assert.rejects(verifyDist(dist), /Expected a production artifact/);
      const html = await readFile(
        join(dist, "blog/posts/interactive-article-example/index.html"),
        "utf8",
      );
      assert.match(html, /How many connections/);
      assert.match(html, /<script[^>]+src="[^"]+\.js"/);
      assert.doesNotMatch(html, /<script[^>]*>\s*[^<\s]/);
      const index = await readFile(join(dist, "blog/index.html"), "utf8");
      assert.ok(index.includes(`href="${base}blog/feed.xml"`));
      assert.match(await readFile(join(dist, "index.html"), "utf8"), /Site home/);
    }
    // A reviewed module must not pull unapproved repository files into its bundle.
    const source = join(root, "scripts/fixtures/blog/example/source.html");
    await writeFile(
      source,
      (await readFile(source, "utf8")).replace(
        '<script type="module">',
        '<script type="module">import "../../../../unapproved.mjs";',
      ),
    );
    await writeFile(join(root, "unapproved.mjs"), "console.log('unapproved')");
    await assert.rejects(bundle("/tmt/"), /Article modules cannot import dependencies/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
