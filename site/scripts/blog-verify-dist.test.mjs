import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { verifyDist as verifyArtifact } from "../src/blog/verify-dist.mjs";

const verifyDist = (root, options) => verifyArtifact(root, { blogDirectory: "", ...options });

// A frozen independent artifact: expected behavior does not use the generator.
const policy =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const head = `<meta http-equiv="Content-Security-Policy" content="${policy}">`;
const fixture = {
  "index.html": `<!doctype html><html><head>${head}</head><body><h1>Index</h1><a href="/blog/">Home</a><a href="/blog/posts/one/">One</a></body></html>`,
  "posts/one/index.html": `<!doctype html><html><head>${head}</head><body><h1>One</h1><article id="blog-article"><p>Complete article body.</p></article><script type="module" src="/blog/assets/one.js"></script></body></html>`,
  "assets/one.js": 'console.log("reviewed module")',
  "feed.xml":
    '<?xml version="1.0"?><rss version="2.0"><channel><title>Blog</title><link>https://example.com/blog/</link><item><title>One</title><link>https://example.com/blog/posts/one/</link></item></channel></rss>',
  "sitemap.xml":
    '<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>https://example.com/blog/posts/one/</loc></url></urlset>',
  "robots.txt": "User-agent: *\nAllow: /blog/\n",
  ".nojekyll": "",
  "build-info.json": JSON.stringify({ demo: false, base: "/blog/", articles: ["one"] }),
};

async function world(changes, run) {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-artifact-test-"));
  try {
    for (const [path, bytes] of Object.entries({ ...fixture, ...changes })) {
      if (bytes === null) continue;
      await mkdir(dirname(join(root, path)), { recursive: true });
      await writeFile(join(root, path), bytes);
    }
    await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("built artifact accepts deployment-root and article links with complete HTML", async () =>
  world({}, async (root) => {
    await verifyDist(root);
  }));

test("public article slugs and generated chunks may contain conversations", async () => {
  const slug = "agent-conversations";
  const chunk = "conversations-3fa9.js";
  await world(
    {
      "index.html": fixture["index.html"].replaceAll("posts/one/", `posts/${slug}/`),
      "posts/one/index.html": null,
      [`posts/${slug}/index.html`]: fixture["posts/one/index.html"].replace(
        "assets/one.js",
        `assets/${chunk}`,
      ),
      "assets/one.js": null,
      [`assets/${chunk}`]: fixture["assets/one.js"],
      "feed.xml": fixture["feed.xml"].replaceAll("posts/one/", `posts/${slug}/`),
      "sitemap.xml": fixture["sitemap.xml"].replaceAll("posts/one/", `posts/${slug}/`),
      "build-info.json": JSON.stringify({ demo: false, base: "/blog/", articles: [slug] }),
    },
    async (root) => verifyDist(root),
  );
});

test("private output is rejected by exact basename or private directory segment", async () => {
  for (const path of [
    "conversations.json",
    "notes/conversations.md",
    "posts/one/conversations/thread.json",
    "manifest.json",
    "posts/one/approval.json",
    "posts/one/meta.json",
    "source.html",
    "nested/.private/draft.txt",
  ]) {
    await world({ [path]: "private source" }, async (root) =>
      assert.rejects(verifyDist(root), /Private source/),
    );
  }
});

test("production verification is default and demo verification requires an explicit opt-in", async () => {
  const demo = {
    "index.html": fixture["index.html"].replace(
      "</head>",
      '<meta name="robots" content="noindex,nofollow"></head>',
    ),
    "posts/one/index.html": fixture["posts/one/index.html"].replace(
      "</head>",
      '<meta name="robots" content="noindex,nofollow"></head>',
    ),
    "build-info.json": JSON.stringify({
      demo: true,
      development: false,
      base: "/blog/",
      articles: ["one"],
    }),
    "feed.xml":
      '<?xml version="1.0"?><rss version="2.0"><channel><title>Preview</title><link>https://example.com/blog/</link></channel></rss>',
    "sitemap.xml":
      '<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"></urlset>',
    "robots.txt": "User-agent: *\nDisallow: /\n",
  };
  await world(demo, async (root) => {
    await assert.rejects(verifyDist(root), /Expected a production artifact/);
    await verifyDist(root, { demo: true });
  });
  await world({}, async (root) => {
    await verifyDist(root);
    await assert.rejects(verifyDist(root, { demo: true }), /Expected an explicit demo artifact/);
  });
  for (const allowedDemo of [false, true]) {
    await world(
      {
        ...demo,
        "build-info.json": JSON.stringify({
          demo: allowedDemo,
          development: true,
          base: "/blog/",
          articles: ["one"],
        }),
      },
      async (root) => assert.rejects(verifyDist(root, { demo: allowedDemo }), /Development policy/),
    );
  }
});

test("data navigation URLs fail while bundled data images remain valid", async () => {
  await world(
    {
      "index.html": fixture["index.html"].replace('href="/blog/"', 'href="data:text/html,example"'),
    },
    async (root) => assert.rejects(verifyDist(root), /Data URL is not an image source/),
  );
  await world(
    {
      "index.html": fixture["index.html"].replace(
        "</body>",
        '<img alt="Example" src="data:image/png;base64,iVBORw0KGgo="></body>',
      ),
    },
    async (root) => verifyDist(root),
  );
});

test("built artifact rejects missing assets, malformed RSS, empty shells and source leakage", async () => {
  const page = fixture["posts/one/index.html"];
  const cases = [
    [
      {
        "build-info.json": JSON.stringify({
          demo: false,
          development: true,
          base: "/blog/",
          articles: ["one"],
        }),
      },
      /Development policy/,
    ],
    [
      { "posts/one/index.html": page.replace("/blog/assets/one.js", "/blog/assets/missing.js") },
      /Missing target/,
    ],
    [{ "feed.xml": '<?xml version="1.0"?><rss><channel></rss>' }, /unexpected close tag/],
    [
      {
        "posts/one/index.html": page.replace(
          "<p>Complete article body.</p>",
          '<div id="root"></div>',
        ),
      },
      /empty SPA shell/,
    ],
    [
      {
        "posts/one/index.html": page.replace(
          '<script type="module" src="/blog/assets/one.js"></script>',
          "<script>alert(1)</script>",
        ),
      },
      /Inline script/,
    ],
    [
      {
        "posts/one/index.html": page.replace(
          'src="/blog/assets/one.js"',
          'src="data:text/javascript,alert(1)"',
        ),
      },
      /Non-local script/,
    ],
    [
      {
        "posts/one/index.html": page.replace(
          "<p>Complete article body.</p>",
          '<p onclick="alert(1)">Complete article body.</p>',
        ),
      },
      /Inline event/,
    ],
    [{ "posts/one/index.html": page.replace(head, "") }, /Missing CSP/],
    [{ "conversations.json": "private" }, /Private source/],
    [
      { "feed.xml": fixture["feed.xml"].replace("/blog/posts/one/", "/blog/posts/missing/") },
      /Missing target/,
    ],
  ];
  for (const [changes, reason] of cases)
    await world(changes, async (root) => assert.rejects(verifyDist(root), reason));
});

test("one Pages artifact resolves blog links to shared assets and the site home", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-shared-artifact-"));
  try {
    for (const [path, bytes] of Object.entries(fixture)) {
      const output = path.startsWith("assets/") ? path : "blog/" + path;
      let content = bytes;
      if (path === "build-info.json")
        content = JSON.stringify({
          demo: false,
          siteBase: "/tmt/",
          base: "/tmt/blog/",
          articles: ["one"],
        });
      else if (path.endsWith(".html") || path.endsWith(".xml"))
        content = bytes
          .replaceAll("/blog/", "/tmt/blog/")
          .replaceAll("/tmt/blog/assets/", "/tmt/assets/");
      if (path === "index.html")
        content = content.replace("</body>", '<a href="/tmt/">Site home</a></body>');
      await mkdir(dirname(join(root, output)), { recursive: true });
      await writeFile(join(root, output), content);
    }
    await writeFile(join(root, "index.html"), "<h1>Existing site</h1>");
    await verifyArtifact(root);
    await rm(join(root, "assets/one.js"));
    await assert.rejects(verifyArtifact(root), /Missing target/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
