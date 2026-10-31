import test from "node:test";
import assert from "node:assert/strict";
import { cp, mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generate } from "../src/blog/build.mjs";
import { readPosts } from "../src/blog/site.mjs";
import { artifactHash, extractArticle, metadata, stableJson } from "../src/blog/content.mjs";

// Synthetic variants exist only in this temporary fixture; they are not translated public posts.
const variants = [
  ["ja", "ja"],
  ["zh-hant", "zh-Hant"],
  ["zh-hans", "zh-Hans"],
];
const slug = "hello-from-tmt";
const origin = "https://example.com";
async function world(run) {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-languages-"));
  try {
    // Copy only the fixture post so page and feed counts do not depend on later posts.
    for (const path of ["src/blog", `src/posts/${slug}`, "scripts/fixtures/blog"])
      await cp(path, join(root, path), { recursive: true });
    const english = join(root, "src/posts", slug);
    const original = await readFile(join(english, "article.html"));
    const meta = JSON.parse(await readFile(join(english, "meta.json"), "utf8"));
    const approval = JSON.parse(await readFile(join(english, "approval.json"), "utf8"));
    for (const [language] of variants) {
      const directory = join(english, language);
      await mkdir(directory);
      const translated = {
        ...meta,
        language,
        title: `Fixture ${language}`,
        summary: `Summary ${language}`,
      };
      const { html } = extractArticle(
        `<article id="blog-article"><p>Fixture body ${language}.</p></article>`,
      );
      await writeFile(join(directory, "article.html"), html);
      await writeFile(join(directory, "meta.json"), stableJson(translated));
      await writeFile(
        join(directory, "approval.json"),
        stableJson({ ...approval, artifactSha256: artifactHash(translated, html, {}) }),
      );
    }
    await run({ root, english, original });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("approved variants have separate routes, language tags, reciprocal alternates and feeds", async () =>
  world(async ({ root, english, original }) => {
    for (const base of ["/", "/tmt/", "/custom/path/"]) {
      const { output, publicFiles } = await generate({ root, base, origin });
      const prefix = `${origin}${base}blog/`;
      const pages = JSON.parse(publicFiles["build-info.json"]).pages;
      assert.equal(pages.length, 8);
      assert.equal(
        pages.some((page) => page.fallbackTo),
        false,
      );
      for (const [language, tag] of [["en", "en"], ...variants]) {
        const directory = language === "en" ? "" : `${language}/`;
        const html = await readFile(join(output, directory, "posts", slug, "index.html"), "utf8");
        assert.ok(html.includes(`<html lang="${tag}">`));
        assert.ok(html.includes(`rel="canonical" href="${prefix}${directory}posts/${slug}/"`));
        assert.match(html, /aria-current="page"/);
        assert.doesNotMatch(html, /translation is not available|noindex/);
        for (const [id, hreflang] of [["en", "en"], ...variants]) {
          const target = id === "en" ? "" : `${id}/`;
          assert.ok(
            html.includes(`hreflang="${hreflang}" href="${prefix}${target}posts/${slug}/"`),
          );
        }
        assert.ok(html.includes(`hreflang="x-default" href="${prefix}posts/${slug}/"`));
        const index = await readFile(join(output, directory, "index.html"), "utf8");
        assert.ok(index.includes(`<html lang="${tag}">`));
        assert.ok(index.includes(`rel="canonical" href="${prefix}${directory}"`));
        assert.doesNotMatch(index, /noindex/);
        assert.ok(publicFiles["sitemap.xml"].includes(`<loc>${prefix}${directory}</loc>`));
        for (const [, indexTag] of variants) assert.ok(index.includes(`hreflang="${indexTag}"`));
        assert.ok(index.includes(`class="article-row" lang="${tag}"`));
        assert.ok(index.includes(`<h3><a href="${base}blog/${directory}posts/${slug}/"`));
        const feed = publicFiles[directory + "feed.xml"];
        assert.ok(feed.includes(`<language>${tag}</language>`));
        assert.ok(feed.includes(`<link>${prefix}${directory}posts/${slug}/</link>`));
        assert.equal((feed.match(/<item>/g) ?? []).length, 1);
        assert.ok(
          publicFiles["sitemap.xml"].includes(`<loc>${prefix}${directory}posts/${slug}/</loc>`),
        );
      }
    }
    assert.deepEqual(await readFile(join(english, "article.html")), original);
  }));

test("missing translations render English with a visible notice but stay out of alternates and indexing", async () =>
  world(async ({ root, english }) => {
    await rm(join(english, "zh-hans"), { recursive: true });
    const { output, publicFiles } = await generate({ root, base: "/tmt/", origin });
    const html = await readFile(join(output, "zh-hans/posts", slug, "index.html"), "utf8");
    assert.match(
      html,
      /Simplified Chinese translation is not available yet. Showing the English article/,
    );
    assert.match(html, /<html lang="en">/);
    assert.match(html, /name="robots" content="noindex,follow"/);
    assert.ok(html.includes(`rel="canonical" href="${origin}/tmt/blog/posts/${slug}/"`));
    assert.doesNotMatch(html, /hreflang="zh-Hans"/);
    assert.match(html, /title="Simplified Chinese \(English fallback\)"/);
    const index = await readFile(join(output, "zh-hans/index.html"), "utf8");
    assert.match(index, /English · translation not available yet/);
    assert.match(index, /<html lang="en">/);
    assert.match(index, /name="robots" content="noindex,follow"/);
    assert.ok(index.includes(`rel="canonical" href="${origin}/tmt/blog/"`));
    const englishIndex = await readFile(join(output, "index.html"), "utf8");
    assert.doesNotMatch(englishIndex, /hreflang="zh-Hans"/);
    assert.ok(!publicFiles["sitemap.xml"].includes(`<loc>${origin}/tmt/blog/zh-hans/</loc>`));
    assert.doesNotMatch(publicFiles["zh-hans/feed.xml"], /<item>/);
    assert.ok(!publicFiles["sitemap.xml"].includes(`/zh-hans/posts/${slug}/`));
    const pages = JSON.parse(publicFiles["build-info.json"]).pages;
    assert.equal(pages.find((page) => page.path === "zh-hans/").fallbackTo, "");
    assert.equal(
      pages.find((page) => page.path === `zh-hans/posts/${slug}/`).fallbackTo,
      `posts/${slug}/`,
    );
  }));

test("English-only and empty blogs expose no indexable foreign-language indexes", async () =>
  world(async ({ root, english }) => {
    for (const [language] of variants) await rm(join(english, language), { recursive: true });
    for (const empty of [false, true]) {
      if (empty) await rm(english, { recursive: true });
      const { output, publicFiles } = await generate({ root, base: "/tmt/", origin });
      const sitemap = publicFiles["sitemap.xml"];
      const englishIndex = await readFile(join(output, "index.html"), "utf8");
      for (const [language, tag] of variants) {
        const html = await readFile(join(output, language, "index.html"), "utf8");
        assert.match(html, /<html lang="en">/);
        assert.match(html, /name="robots" content="noindex,follow"/);
        assert.ok(html.includes(`rel="canonical" href="${origin}/tmt/blog/"`));
        assert.doesNotMatch(englishIndex, new RegExp(`hreflang="${tag}"`));
        assert.ok(!sitemap.includes(`<loc>${origin}/tmt/blog/${language}/</loc>`));
        assert.doesNotMatch(publicFiles[language + "/feed.xml"], /<item>/);
      }
      assert.doesNotMatch(englishIndex, /noindex/);
    }
  }));

test("language and approval bindings reject misplaced or reused variants", async () =>
  world(async ({ root, english }) => {
    for (const language of ["en", ...variants.map(([id]) => id)])
      assert.equal(
        metadata({ slug, title: "Fixture", summary: "Fixture", date: "2026-10-25", language })
          .language,
        language,
      );
    const path = join(english, "ja/meta.json");
    const meta = JSON.parse(await readFile(path, "utf8"));
    await writeFile(path, stableJson({ ...meta, language: "zh-hant" }));
    await assert.rejects(readPosts(join(root, "src/posts")), /directory must match language/);
    await writeFile(path, stableJson(meta));
    await cp(join(english, "approval.json"), join(english, "ja/approval.json"));
    await assert.rejects(readPosts(join(root, "src/posts")), /artifact hash mismatch/);
  }));
