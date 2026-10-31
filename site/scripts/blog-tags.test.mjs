import test from "node:test";
import assert from "node:assert/strict";
import { cp, mkdtemp, mkdir, readFile, writeFile, rm, stat } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { build } from "vite-plus";
import { metadata, artifactHash, sha256, stableJson } from "../src/blog/content.mjs";
import { postTags } from "../src/blog/tags.mjs";
import { indexBody, postBody, rss, locationConfig } from "../src/blog/site.mjs";
import { generate, blogBuild } from "../src/blog/build.mjs";
import { prunePages } from "../src/blog/pages.mjs";
import { verifyDist } from "../src/blog/verify-dist.mjs";

const meta = {
  slug: "tagged-post",
  title: "Tagged post",
  summary: "Fixture",
  date: "2026-10-26",
  language: "en",
};
const article = '<article id="blog-article"><p>Fixture.</p></article>\n';

test("tags use a closed small vocabulary without mutating approved metadata", () => {
  const legacy = { ...meta };
  const before = JSON.stringify(legacy);
  assert.equal(metadata(legacy), legacy);
  assert.equal(JSON.stringify(legacy), before);
  assert.equal(Object.hasOwn(legacy, "tags"), false);
  for (const tags of [[], ...postTags.map((tag) => [tag]), ["design", "colab", "workflow"]]) {
    const tagged = { ...meta, tags };
    assert.equal(metadata(tagged), tagged);
    assert.equal(tagged.tags, tags);
  }
  for (const tags of [
    null,
    "design",
    ["unknown"],
    ["Design"],
    ["../design"],
    ["design", "design"],
    postTags.slice(0, 4),
    ["design", 3],
  ])
    assert.throws(() => metadata({ ...meta, tags }), /tags|tag/);
});

test("tags are part of the exact approved artifact while legacy hashes stay unchanged", () => {
  const legacy = artifactHash(meta, article, {});
  metadata(meta);
  assert.equal(artifactHash(meta, article, {}), legacy);
  assert.notEqual(artifactHash({ ...meta, tags: ["design"] }, article, {}), legacy);
  assert.notEqual(
    artifactHash({ ...meta, tags: ["design"] }, article, {}),
    artifactHash({ ...meta, tags: ["colab"] }, article, {}),
  );
});

test("listing and article tags are separate links and RSS categories", async () => {
  const tagged = { meta: { ...meta, tags: ["design", "colab"] }, html: article, assets: {} };
  const site = { title: "Journal", description: "Notes" };
  for (const base of ["/", "/tmt/", "/custom/path/"]) {
    const location = locationConfig({ base, origin: "https://example.com" });
    const index = indexBody([tagged], site, location);
    assert.doesNotMatch(index, /<a class="article-row"/);
    assert.match(index, /<h3><a href=/);
    for (const html of [index, postBody(tagged, location)])
      for (const tag of tagged.meta.tags)
        assert.ok(html.includes(`href="${base}blog/tags/${tag}/">#${tag}</a>`));
    const feed = rss([tagged], site, location);
    assert.match(feed, /<category>design<\/category><category>colab<\/category>/);
  }
});

// Complete synthetic approved posts; no published post is edited by these tests.
async function taggedWorld(run) {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-tags-"));
  try {
    for (const path of ["src/blog", "scripts/fixtures/blog"])
      await cp(path, join(root, path), { recursive: true });
    await writeFile(
      join(root, "index.html"),
      "<!doctype html><html><body><h1>Site home</h1></body></html>",
    );
    for (const post of [
      { ...meta, tags: ["announcements", "design", "colab"] },
      { ...meta, slug: "other-post", title: "Other post", tags: ["workflow"] },
      { ...meta, language: "ja", title: "Japanese fixture", tags: ["design", "engineering"] },
    ]) {
      const directory = join(
        root,
        "src/posts",
        post.slug,
        post.language === "en" ? "" : post.language,
      );
      await mkdir(directory, { recursive: true });
      await writeFile(join(directory, "article.html"), article);
      await writeFile(join(directory, "meta.json"), stableJson(post));
      await writeFile(
        join(directory, "approval.json"),
        stableJson({
          decision: "approved",
          pageId: "10000000-0000-4000-8000-000000000001",
          proposalId: "20000000-0000-4000-8000-000000000001",
          approverDevice: "30000000-0000-4000-8000-000000000001",
          actionId: "40000000-0000-4000-8000-000000000001",
          sourceSha256: sha256("Synthetic source"),
          pageRevision: "fixture",
          approvedAt: "2026-10-26T00:00:00Z",
          artifactSha256: artifactHash(post, article, {}),
        }),
      );
    }
    await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("used tags produce filtered static indexes and sitemap entries at every site base", async () =>
  taggedWorld(async (root) => {
    for (const base of ["/", "/tmt/", "/custom/path/"]) {
      const { output, publicFiles } = await generate({ root, base, origin: "https://example.com" });
      const html = await readFile(join(output, "tags/design/index.html"), "utf8");
      assert.match(html, /<h1[^>]*>#design<\/h1>/);
      assert.match(html, /Tagged post/);
      assert.doesNotMatch(html, /Other post/);
      assert.ok(
        html.includes(`rel="canonical" href="https://example.com${base}blog/tags/design/"`),
      );
      assert.ok(
        publicFiles["sitemap.xml"].includes(
          `<loc>https://example.com${base}blog/tags/design/</loc>`,
        ),
      );
      assert.match(publicFiles["feed.xml"], /<category>design<\/category>/);
      await assert.rejects(stat(join(output, "tags/engineering/index.html")), { code: "ENOENT" });
      const japanese = await readFile(join(output, "ja/tags/design/index.html"), "utf8");
      assert.match(japanese, /<html lang="ja">/);
      assert.match(japanese, /Japanese fixture/);
      assert.doesNotMatch(japanese, /Tagged post|Other post|noindex/);
      for (const page of [html, japanese]) {
        assert.ok(
          page.includes(`hreflang="en" href="https://example.com${base}blog/tags/design/"`),
        );
        assert.ok(
          page.includes(`hreflang="ja" href="https://example.com${base}blog/ja/tags/design/"`),
        );
        assert.doesNotMatch(page, /hreflang="zh-/);
      }
      const japaneseOnly = await readFile(join(output, "ja/tags/engineering/index.html"), "utf8");
      assert.ok(
        japaneseOnly.includes(
          `rel="canonical" href="https://example.com${base}blog/ja/tags/engineering/"`,
        ),
      );
      assert.match(
        japaneseOnly,
        /<span aria-disabled="true" title="No English articles for #engineering">EN<\/span>/,
      );
      assert.doesNotMatch(japaneseOnly, /hreflang="en"|hreflang="x-default"/);
      assert.ok(
        publicFiles["sitemap.xml"].includes(
          `<loc>https://example.com${base}blog/ja/tags/engineering/</loc>`,
        ),
      );
      assert.ok(!publicFiles["sitemap.xml"].includes(`${base}blog/tags/engineering/`));
      assert.match(
        publicFiles["ja/feed.xml"],
        /<category>design<\/category><category>engineering<\/category>/,
      );
      for (const path of ["zh-hant/index.html", "zh-hant/posts/tagged-post/index.html"]) {
        const fallback = await readFile(join(output, path), "utf8");
        assert.ok(fallback.includes(`href="${base}blog/tags/design/">#design</a>`));
        assert.ok(!fallback.includes(`${base}blog/zh-hant/tags/`));
      }
      await assert.rejects(stat(join(output, "zh-hant/tags/design/index.html")), {
        code: "ENOENT",
      });
    }
  }));

test("built tag pages verify CSP, canonical and assets; unknown tags fail before building", async () =>
  taggedWorld(async (root) => {
    const origin = "https://example.com";
    const blog = await blogBuild({
      root,
      base: "/tmt/",
      origin,
      repository: "build-owner/project",
      development: false,
    });
    await build({
      configFile: false,
      root,
      base: "/tmt/",
      logLevel: "silent",
      plugins: [blog.plugin],
      build: { rolldownOptions: { input: [join(root, "index.html"), ...blog.entries] } },
    });
    const dist = join(root, "dist");
    await verifyDist(dist, { origin });
    const pages = join(root, "dist-pages");
    await prunePages(dist, pages, { origin, repository: "build-owner/project" });
    await verifyDist(pages, { origin, repository: "build-owner/project", pages: true });
    const tagPage = join(dist, "blog/tags/design/index.html");
    const html = await readFile(tagPage, "utf8");
    await writeFile(
      tagPage,
      html.replace(
        'rel="canonical" href="https://example.com/tmt/blog/tags/design/"',
        'rel="canonical" href="https://wrong.example/"',
      ),
    );
    await assert.rejects(verifyDist(dist, { origin }), /Canonical link/);
    await writeFile(tagPage, html);
    const feedPath = join(dist, "blog/feed.xml");
    const feed = await readFile(feedPath, "utf8");
    await writeFile(
      feedPath,
      feed.replace("<category>design</category>", "<category>unknown</category>"),
    );
    await assert.rejects(verifyDist(dist, { origin }), /Unknown RSS tag/);
    await writeFile(feedPath, feed);
    await rm(tagPage);
    await assert.rejects(verifyDist(dist, { origin }), /HTML count mismatch|Missing target/);
    const metaPath = join(root, "src/posts/tagged-post/meta.json");
    const saved = JSON.parse(await readFile(metaPath, "utf8"));
    await writeFile(metaPath, JSON.stringify({ ...saved, tags: ["unknown"] }));
    await assert.rejects(
      blogBuild({
        root,
        base: "/tmt/",
        origin,
        repository: "build-owner/project",
        development: false,
      }),
      /Unknown post tag/,
    );
  }));
