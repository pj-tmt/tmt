import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { verifyDist as verifyArtifact } from "../src/blog/verify-dist.mjs";

// The origin the fixture's canonical links, RSS and sitemap name.
const origin = "https://example.com";
const verifyDist = (root, options) =>
  verifyArtifact(root, { blogDirectory: "", origin, ...options });

// A frozen independent artifact: expected behavior does not use the generator.
const policy =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const head = `<meta http-equiv="Content-Security-Policy" content="${policy}">`;
const fixture = {
  "index.html": `<!doctype html><html lang="en"><head>${head}<link rel="canonical" href="${origin}/blog/"></head><body><h1>Index</h1><a href="/blog/">Home</a><a href="/blog/posts/one/">One</a></body></html>`,
  "posts/one/index.html": `<!doctype html><html lang="en"><head>${head}<link rel="canonical" href="${origin}/blog/posts/one/"></head><body><h1>One</h1><article id="blog-article"><p>Complete article body.</p></article><script type="module" src="/blog/assets/one.js"></script></body></html>`,
  "assets/one.js": 'console.log("reviewed module")',
  "feed.xml":
    '<?xml version="1.0"?><rss version="2.0"><channel><title>Blog</title><link>https://example.com/blog/</link><item><title>One</title><link>https://example.com/blog/posts/one/</link></item></channel></rss>',
  "sitemap.xml":
    '<?xml version="1.0"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>https://example.com/blog/posts/one/</loc></url></urlset>',
  "robots.txt": "User-agent: *\nAllow: /blog/\n",
  ".nojekyll": "",
  "build-info.json": JSON.stringify({
    demo: false,
    base: "/blog/",
    pages: [
      { path: "", language: "en" },
      { path: "posts/one/", language: "en" },
    ],
  }),
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
      [`posts/${slug}/index.html`]: fixture["posts/one/index.html"]
        .replaceAll("posts/one/", `posts/${slug}/`)
        .replace("assets/one.js", `assets/${chunk}`),
      "assets/one.js": null,
      [`assets/${chunk}`]: fixture["assets/one.js"],
      "feed.xml": fixture["feed.xml"].replaceAll("posts/one/", `posts/${slug}/`),
      "sitemap.xml": fixture["sitemap.xml"].replaceAll("posts/one/", `posts/${slug}/`),
      "build-info.json": JSON.stringify({
        demo: false,
        base: "/blog/",
        pages: [
          { path: "", language: "en" },
          { path: `posts/${slug}/`, language: "en" },
        ],
      }),
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
      pages: [
        { path: "", language: "en" },
        { path: "posts/one/", language: "en" },
      ],
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
    // A demo artifact names no origin, so none is asked for.
    await verifyArtifact(root, { blogDirectory: "", demo: true });
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
          pages: [
            { path: "", language: "en" },
            { path: "posts/one/", language: "en" },
          ],
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
          pages: [
            { path: "", language: "en" },
            { path: "posts/one/", language: "en" },
          ],
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

// The blog inside a project-site Pages artifact: shared assets and the site home live beside blog/.
async function sharedArtifact(root) {
  for (const [path, bytes] of Object.entries(fixture)) {
    const output = path.startsWith("assets/") ? path : "blog/" + path;
    let content = bytes;
    if (path === "build-info.json")
      content = JSON.stringify({
        demo: false,
        siteBase: "/tmt/",
        base: "/tmt/blog/",
        pages: [
          { path: "", language: "en" },
          { path: "posts/one/", language: "en" },
        ],
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
}

test("one Pages artifact resolves blog links to shared assets and the site home", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-shared-artifact-"));
  try {
    await sharedArtifact(root);
    await verifyArtifact(root, { origin });
    await rm(join(root, "assets/one.js"));
    await assert.rejects(verifyArtifact(root, { origin }), /Missing target/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a production artifact must name the origin it was given, in canonical links, RSS and sitemap", async () => {
  const other = "https://old.example.org";
  const page = fixture["posts/one/index.html"];
  const cases = [
    [
      { "posts/one/index.html": page.replace(origin, other) },
      /Canonical link of posts\/one\/index\.html must be https:\/\/example\.com\/blog\/posts\/one\//,
    ],
    [{ "posts/one/index.html": page.replace(/<link rel="canonical"[^>]*>/, "") }, /found null/],
    [
      {
        "index.html": fixture["index.html"].replace(
          'href="https://example.com/blog/"',
          'href="/blog/"',
        ),
      },
      /Canonical link of index\.html/,
    ],
    [
      { "feed.xml": fixture["feed.xml"].replaceAll(origin, other) },
      /feed\.xml names https:\/\/old\.example\.org/,
    ],
    [
      { "sitemap.xml": fixture["sitemap.xml"].replaceAll(origin, other) },
      /sitemap\.xml names https:\/\/old\.example\.org/,
    ],
  ];
  for (const [changes, reason] of cases)
    await world(changes, async (root) => assert.rejects(verifyDist(root), reason));
  await world({}, async (root) => {
    // The same artifact fails for any other origin, and no origin is no verdict.
    await assert.rejects(verifyDist(root, { origin: other }), /Canonical link/);
    await assert.rejects(verifyArtifact(root, { blogDirectory: "" }), /SITE_ORIGIN is required/);
    for (const bad of ["http://example.com", "https://example.com/blog/", "example.com"])
      await assert.rejects(
        verifyDist(root, { origin: bad }),
        /SITE_ORIGIN must be an HTTPS origin/,
      );
  });
});

test("the command reads the origin from SITE_ORIGIN and refuses a production artifact without it", async () => {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-command-"));
  try {
    await sharedArtifact(join(root, "dist"));
    const script = fileURLToPath(new URL("../src/blog/verify-dist.mjs", import.meta.url));
    const run = (env) =>
      spawnSync(process.execPath, [script], {
        cwd: root,
        encoding: "utf8",
        env: { PATH: process.env.PATH, ...env },
      });
    const verified = run({ SITE_ORIGIN: origin });
    assert.equal(verified.status, 0, verified.stderr);
    assert.match(verified.stdout, /Verified 2 rendered HTML pages/);
    const wrong = run({ SITE_ORIGIN: "https://old.example.org" });
    assert.notEqual(wrong.status, 0);
    assert.match(wrong.stderr, /Canonical link of index\.html must be https:\/\/old\.example\.org/);
    for (const missing of [{}, { SITE_ORIGIN: "" }]) {
      const result = run(missing);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /SITE_ORIGIN is required/);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
