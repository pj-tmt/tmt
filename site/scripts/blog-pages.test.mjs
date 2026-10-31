import test from "node:test";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { prunePages } from "../src/blog/pages.mjs";
import { verifyDist } from "../src/blog/verify-dist.mjs";
import { filesBelow } from "../src/blog/site.mjs";

const origin = "https://example.com";
const repository = "new-owner/project";
const options = { origin, repository, pages: true };
// Independent production-shaped fixture, including resources used only by Home.
const policy =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
async function world(base, run) {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-pages-"));
  const source = join(root, "dist");
  const output = join(root, "dist-pages");
  const blog = base + "blog/";
  const pages = [
    { path: "", language: "en" },
    { path: "posts/one/", language: "en" },
    { path: "tags/design/", language: "en" },
    { path: "ja/", language: "ja" },
    { path: "ja/posts/one/", language: "ja" },
    { path: "ja/tags/design/", language: "ja" },
  ];
  const files = {
    "index.html": '<h1>Home</h1><script src="./assets/home.js"></script>',
    "404.html": '<div id="root">Home fallback</div>',
    "ja/index.html": "<h1>Japanese Home</h1>",
    "colab/index.html": "<h1>Colab</h1>",
    "working/index.html": "<h1>Old chapter redirect</h1>",
    "assets/home.js": "console.log('Home')",
    "assets/unused.css": "body{color:red}",
    "assets/blog.css": 'body{background:url("./texture.png")} @import "./theme.css";',
    "assets/theme.css": "body{color:white}",
    "assets/texture.png": "fixture bytes",
    "assets/article.js":
      'import "./shared.js"; export {x} from "./reexport.js"; import("./lazy.js"); new URL("./image.png", import.meta.url);',
    "assets/shared.js": "console.log('shared')",
    "assets/reexport.js": "export const x = 1;",
    "assets/lazy.js": "console.log('lazy')",
    "assets/image.png": "fixture image",
    "blog/mark.svg": '<svg xmlns="http://www.w3.org/2000/svg"/>',
    "blog/.nojekyll": "",
    "blog/robots.txt": `User-agent: *\nAllow: ${blog}\n`,
    "blog/feed.xml": `<?xml version="1.0"?><rss version="2.0"><channel><link>${origin}${blog}</link></channel></rss>`,
    "blog/ja/feed.xml": `<?xml version="1.0"?><rss version="2.0"><channel><link>${origin}${blog}ja/</link></channel></rss>`,
    "blog/sitemap.xml": `<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${pages.map(({ path }) => `<url><loc>${origin}${blog}${path}</loc></url>`).join("")}</urlset>`,
    "blog/build-info.json": JSON.stringify({
      demo: false,
      development: false,
      siteBase: base,
      base: blog,
      repository: `https://github.com/${repository}`,
      pages,
    }),
  };
  for (const page of pages)
    files[`blog/${page.path}index.html`] =
      `<!doctype html><html lang="${page.language}"><head><meta http-equiv="Content-Security-Policy" content="${policy}"><link rel="canonical" href="${origin}${blog}${page.path}"><link rel="stylesheet" href="${base}assets/blog.css"></head><body><header class="site-header"><a href="https://github.com/${repository}">Explore TMT ↗</a></header><h1>Journal</h1><a href="${blog}">Articles</a><img src="${blog}mark.svg" alt=""><article id="blog-article"><p>Fixture article body.</p></article><script type="module" src="${base}assets/article.js"></script><footer class="site-footer"><a href="https://github.com/${repository}">TMT ↗</a></footer></body></html>`;
  try {
    for (const [path, bytes] of Object.entries(files)) {
      await mkdir(dirname(join(source, path)), { recursive: true });
      await writeFile(join(source, path), bytes);
    }
    await run({ source, output, files, base, blog });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("Pages retains language/topic routes and transitive assets while pruning Home at every base", async () => {
  for (const base of ["/", "/tmt/", "/custom/path/"])
    await world(base, async ({ source, output, files, blog }) => {
      const original = await filesBelow(source);
      const listing = await prunePages(source, output, options);
      await verifyDist(output, options);
      const actual = await filesBelow(output);
      assert.deepEqual(await filesBelow(source), original);
      const expected = Object.keys(files).filter(
        (path) =>
          path.startsWith("blog/") ||
          (path.startsWith("assets/") && !["assets/home.js", "assets/unused.css"].includes(path)),
      );
      assert.deepEqual(listing, [...expected, "index.html", "404.html"].sort());
      for (const path of expected) assert.deepEqual(actual[path], original[path]);
      assert.match(actual["index.html"].toString(), new RegExp(`content="0;url=${blog}"`));
      assert.ok(actual["404.html"].toString().includes(`href="https://github.com/${repository}"`));
      await writeFile(join(output, "assets/stale.js"), "stale");
      await prunePages(source, output, options);
      assert.deepEqual(await filesBelow(output), actual);
    });
});

test("Pages rejects leaked HTML/assets, broken dependency closure, wrong repository and broken entry links", async () =>
  world("/tmt/", async ({ source, output, files }) => {
    await prunePages(source, output, options);
    const cases = [
      ["ja/index.html", "<h1>Leaked locale Home</h1>", /Non-blog HTML/],
      ["assets/home.js", "console.log('leak')", /Unreferenced file/],
      ["assets/shared.js", null, /Missing Pages target/],
      ["assets/texture.png", null, /Missing Pages target/],
      ["index.html", "<a href='/tmt/blog/'>Blog</a>", /root redirect/],
      ["404.html", "<a href='/tmt/blog/'>Blog</a>", /404 links/],
      [
        "blog/index.html",
        files["blog/index.html"].replaceAll(
          `https://github.com/${repository}`,
          "https://github.com/wrong/repo",
        ),
        /header\/footer/,
      ],
    ];
    for (const [path, bytes, error] of cases) {
      const previous = await readFile(join(output, path)).catch(() => null);
      await mkdir(dirname(join(output, path)), { recursive: true });
      if (bytes === null) await rm(join(output, path));
      else await writeFile(join(output, path), bytes);
      await assert.rejects(verifyDist(output, options), error);
      if (previous === null) await rm(join(output, path));
      else await writeFile(join(output, path), previous);
      await verifyDist(output, options);
    }
    await assert.rejects(
      verifyDist(output, { ...options, repository: "other/repo" }),
      /Pages repository/,
    );
    const previous = await filesBelow(output);
    for (const flag of ["demo", "development"]) {
      const info = JSON.parse(files["blog/build-info.json"]);
      info[flag] = true;
      await writeFile(join(source, "blog/build-info.json"), JSON.stringify(info));
      await assert.rejects(prunePages(source, output, options), /requires a production blog build/);
      assert.deepEqual(await filesBelow(output), previous);
    }
    await writeFile(join(source, "blog/build-info.json"), files["blog/build-info.json"]);
    await rm(join(source, "assets/lazy.js"));
    await assert.rejects(prunePages(source, output, options), /Missing Pages target/);
    assert.deepEqual(await filesBelow(output), previous);
    await assert.rejects(prunePages(source, source, options), /must be separate/);
    await assert.rejects(prunePages(source, join(source, "pages"), options), /must be separate/);
  }));

test("the Pages commands build and verify the exact deployment directory", async () =>
  world("/tmt/", async ({ source, output }) => {
    const run = (script, args = []) =>
      spawnSync(
        process.execPath,
        [fileURLToPath(new URL(`../src/blog/${script}.mjs`, import.meta.url)), ...args],
        {
          cwd: dirname(source),
          timeout: 15_000,
          encoding: "utf8",
          env: { PATH: process.env.PATH, SITE_ORIGIN: origin, GITHUB_REPOSITORY: repository },
        },
      );
    const built = run("pages");
    assert.equal(built.status, 0, built.stderr);
    assert.match(built.stdout, /Blog-only Pages artifact/);
    const verified = run("verify-dist", ["--pages"]);
    assert.equal(verified.status, 0, verified.stderr);
    await verifyDist(output, options);
  }));
