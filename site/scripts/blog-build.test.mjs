import test from "node:test";
import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build, createServer } from "vite-plus";
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
      for (const page of [index, html]) {
        assert.ok(page.includes(`src="${base}blog/mark.svg"`));
        assert.ok(page.includes(`href="${base}blog/favicon.png"`));
      }
      assert.equal(
        await readFile(join(dist, "blog/mark.svg"), "utf8"),
        await readFile(join(root, "src/blog/mark.svg"), "utf8"),
      );
      assert.ok(
        (await readFile(join(dist, "blog/favicon.png"))).equals(
          await readFile(join(root, "src/blog/favicon.png")),
        ),
      );
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

test("dev serves the file-backed mark and favicon at exactly one site base on index and post pages", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-dev-mark-"));
  try {
    for (const path of ["src/blog", "src/posts", "scripts/fixtures/blog"])
      await cp(path, join(root, path), { recursive: true });
    const mark = await readFile(join(root, "src/blog/mark.svg"), "utf8");
    const favicon = await readFile(join(root, "src/blog/favicon.png"));
    for (const base of ["/", "/tmt/", "/custom/path/"]) {
      const blog = await blogBuild({ root, base, demo: true, development: true });
      const server = await createServer({
        configFile: false,
        root,
        base,
        logLevel: "silent",
        plugins: [blog.plugin],
        server: { host: "127.0.0.1", port: 0, watch: null },
      });
      try {
        await server.listen();
        const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
        for (const path of ["", "posts/interactive-article-example/"]) {
          const url = `${origin}${base}blog/${path}`;
          const response = await fetch(url);
          assert.equal(response.status, 200);
          const html = await response.text();
          const image = html.match(/<img[^>]+src="([^"]+)"/)[1];
          const icon = html.match(/<link[^>]+rel="icon"[^>]+href="([^"]+)"/)[1];
          assert.equal(new URL(image, url).pathname, `${base}blog/mark.svg`);
          const fetchedMark = await fetch(new URL(image, url));
          assert.equal(fetchedMark.status, 200);
          assert.match(fetchedMark.headers.get("content-type"), /image\/svg\+xml/);
          assert.equal(await fetchedMark.text(), mark);
          assert.equal(new URL(icon, url).pathname, `${base}blog/favicon.png`);
          const fetchedIcon = await fetch(new URL(icon, url));
          assert.equal(fetchedIcon.status, 200);
          assert.match(fetchedIcon.headers.get("content-type"), /image\/png/);
          assert.ok(Buffer.from(await fetchedIcon.arrayBuffer()).equals(favicon));
        }
      } finally {
        await server.close();
      }
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a production build needs SITE_ORIGIN and its artifact names that origin, not the local one", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-origin-"));
  const origin = "https://example.com";
  try {
    for (const path of ["src/blog", "src/posts", "scripts/fixtures/blog"])
      await cp(path, join(root, path), { recursive: true });
    await writeFile(
      join(root, "index.html"),
      "<!doctype html><html><body><h1>Site home</h1></body></html>",
    );
    const bundle = async (options) => {
      const blog = await blogBuild({
        root,
        base: "/tmt/",
        demo: false,
        development: false,
        repository: "build-owner/project",
        ...options,
      });
      return build({
        configFile: false,
        root,
        base: "/tmt/",
        logLevel: "silent",
        plugins: [blog.plugin],
        build: { rolldownOptions: { input: [join(root, "index.html"), ...blog.entries] } },
      });
    };
    await assert.rejects(bundle(), /SITE_ORIGIN is required/);
    await assert.rejects(bundle({ origin, repository: "" }), /must name owner\/repository/);
    await assert.rejects(bundle({ origin: "" }), /SITE_ORIGIN is required/);
    // A development server and a demo build need none.
    await blogBuild({ root, base: "/tmt/", demo: false, development: true });
    await bundle({ origin });
    const dist = join(root, "dist");
    await verifyDist(dist, { origin });
    await assert.rejects(verifyDist(dist, { origin: "https://old.example.org" }), /Canonical link/);
    const fallbackPath = join(dist, "blog/ja/posts/hello-from-tmt/index.html");
    const fallback = await readFile(fallbackPath, "utf8");
    await writeFile(
      fallbackPath,
      fallback.replace(
        'name="robots" content="noindex,follow"',
        'name="robots" content="index,follow"',
      ),
    );
    await assert.rejects(verifyDist(dist, { origin }), /Invalid or indexable fallback/);
    await writeFile(fallbackPath, fallback.replace('<html lang="en">', '<html lang="ja">'));
    await assert.rejects(verifyDist(dist, { origin }), /Wrong HTML language/);
    await writeFile(fallbackPath, fallback);
    await writeFile(fallbackPath, fallback.replace('hreflang="en"', 'hreflang="ja"'));
    await assert.rejects(verifyDist(dist, { origin }), /Invalid language alternate/);
    await writeFile(fallbackPath, fallback);
    const feedPath = join(dist, "blog/ja/feed.xml");
    const feed = await readFile(feedPath, "utf8");
    await writeFile(
      feedPath,
      feed.replace(
        "</channel>",
        `<item><link>${origin}/tmt/blog/ja/posts/hello-from-tmt/</link></item></channel>`,
      ),
    );
    await assert.rejects(verifyDist(dist, { origin }), /Fallback URL in feed\/sitemap/);
    await writeFile(feedPath, feed);
    const indexPath = join(dist, "blog/ja/index.html");
    const fallbackIndex = await readFile(indexPath, "utf8");
    assert.match(fallbackIndex, /<html lang="en">/);
    assert.ok(fallbackIndex.includes(`rel="canonical" href="${origin}/tmt/blog/"`));
    await writeFile(
      indexPath,
      fallbackIndex.replace(
        'name="robots" content="noindex,follow"',
        'name="robots" content="index,follow"',
      ),
    );
    await assert.rejects(verifyDist(dist, { origin }), /Invalid or indexable fallback/);
    await writeFile(indexPath, fallbackIndex);
    const infoPath = join(dist, "blog/build-info.json");
    const inventory = await readFile(infoPath, "utf8");
    const info = JSON.parse(inventory);
    delete info.pages.find((page) => page.path === "ja/").fallbackTo;
    await writeFile(infoPath, JSON.stringify(info));
    await assert.rejects(
      verifyDist(dist, { origin }),
      /Index fallback must match approved language articles/,
    );
    await writeFile(infoPath, inventory);
    const sitemapPath = join(dist, "blog/sitemap.xml");
    const sitemap = await readFile(sitemapPath, "utf8");
    await writeFile(
      sitemapPath,
      sitemap.replace("</urlset>", `<url><loc>${origin}/tmt/blog/ja/</loc></url></urlset>`),
    );
    await assert.rejects(verifyDist(dist, { origin }), /Fallback URL in feed\/sitemap/);
    await writeFile(sitemapPath, sitemap);
    await verifyDist(dist, { origin });
    const index = await readFile(join(dist, "blog/index.html"), "utf8");
    assert.ok(index.includes(`<link rel="canonical" href="${origin}/tmt/blog/">`));
    assert.match(
      await readFile(join(dist, "blog/feed.xml"), "utf8"),
      /<link>https:\/\/example\.com\/tmt\/blog\/<\/link>/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
