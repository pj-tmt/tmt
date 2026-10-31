import { mkdir, readFile, writeFile, rm, copyFile, realpath } from "node:fs/promises";
import { dirname, join } from "node:path";
import { extractArticle, articleFromSource, metadata, stableJson } from "./content.mjs";
import {
  indexBody,
  layout,
  locationConfig,
  postBody,
  readPosts,
  repositoryUrl,
  rss,
} from "./site.mjs";
import { languages, indexPath, postPath } from "./languages.mjs";
import { postTags, tagPath } from "./tags.mjs";

// Generated HTML lives under the Vite root so its output paths are blog/**.
// Only this directory is disposable; approved src/posts and review records are not.
export async function generate({
  root,
  demo = false,
  development = false,
  base,
  origin,
  repository,
}) {
  const location = locationConfig({ base, origin: origin || undefined, repository });
  const site = JSON.parse(await readFile(join(root, "src/blog/site.json"), "utf8"));
  let posts = await readPosts(join(root, "src/posts"));
  if (demo) {
    const fixture = join(root, "scripts/fixtures/blog/example");
    const meta = metadata(JSON.parse(await readFile(join(fixture, "meta.json"), "utf8")));
    const raw = articleFromSource(await readFile(join(fixture, "source.html"), "utf8"));
    posts = [{ meta, ...extractArticle(raw) }];
  }
  const output = join(root, "blog");
  await rm(output, { recursive: true, force: true });
  const write = async (path, bytes) => {
    const target = join(output, path);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, bytes);
  };
  const entries = [];
  const pages = [];
  const english = posts.filter(({ meta }) => meta.language === "en");
  const render = async ({ path, language, canonicalPath = path, ...options }) => {
    entries.push(join(output, path, "index.html"));
    pages.push({
      path,
      language,
      ...(canonicalPath !== path ? { fallbackTo: canonicalPath } : {}),
    });
    await write(
      path + "index.html",
      layout({
        site,
        location,
        demo,
        development,
        path,
        language,
        canonicalPath,
        ...options,
      }),
    );
  };
  const publicFeeds = {};
  const indexAlternates = languages
    .filter(({ id }) => id === "en" || posts.some(({ meta }) => meta.language === id))
    .map(({ id }) => ({ language: id, path: indexPath(id) }));
  for (const language of languages) {
    const translated = posts.filter(({ meta }) => meta.language === language.id);
    const indexFallback = language.id !== "en" && translated.length === 0;
    const visible = english.map(
      (post) => translated.find(({ meta }) => meta.slug === post.meta.slug) ?? post,
    );
    await render({
      path: indexPath(language.id),
      canonicalPath: indexFallback ? "" : indexPath(language.id),
      language: indexFallback ? "en" : language.tag,
      fallback: indexFallback,
      requestedLanguage: language.id,
      title: site.title,
      description: site.description,
      body: indexBody(visible, site, location, language.id, indexFallback),
      alternates: indexAlternates,
    });
    for (const post of visible) {
      const { meta } = post;
      const path = postPath(meta.slug, language.id);
      const canonicalPath = postPath(meta.slug, meta.language);
      const variants = posts.filter((item) => item.meta.slug === meta.slug);
      await render({
        path,
        canonicalPath,
        language: languages.find((item) => item.id === meta.language).tag,
        requestedLanguage: language.id,
        slug: meta.slug,
        fallback: meta.language !== language.id,
        title: `${meta.title} · ${site.title}`,
        description: meta.summary,
        alternates: variants.map(({ meta }) => ({
          language: meta.language,
          path: postPath(meta.slug, meta.language),
        })),
        body: postBody(post, location, language.id),
      });
      // Fallback routes reuse approved English bytes, including their relative asset references.
      for (const [asset, bytes] of Object.entries(post.assets)) await write(path + asset, bytes);
    }
    publicFeeds[indexPath(language.id) + "feed.xml"] = rss(
      demo ? [] : translated,
      site,
      location,
      language.id,
    );
  }
  for (const tag of postTags) {
    const tagged = posts.filter(({ meta }) => meta.tags?.includes(tag));
    const tagLanguages = languages.filter(({ id }) =>
      tagged.some(({ meta }) => meta.language === id),
    );
    const alternates = tagLanguages.map(({ id }) => ({ language: id, path: tagPath(tag, id) }));
    for (const language of tagLanguages)
      await render({
        path: tagPath(tag, language.id),
        language: language.tag,
        requestedLanguage: language.id,
        tag,
        alternates,
        title: `#${tag} · ${site.title}`,
        description: `Articles tagged #${tag}.`,
        body: indexBody(
          tagged.filter(({ meta }) => meta.language === language.id),
          site,
          location,
          language.id,
          false,
          { tag },
        ),
      });
  }
  const publicFiles = {
    ...publicFeeds,
    "robots.txt": demo
      ? "User-agent: *\nDisallow: /\n"
      : `User-agent: *\nAllow: ${location.base}\nSitemap: ${location.url}sitemap.xml\n`,
    "sitemap.xml": `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${(demo ? [] : pages.filter((page) => page.fallbackTo === undefined).map((page) => page.path)).map((path) => `<url><loc>${location.url}${path}</loc></url>`).join("")}</urlset>\n`,
    "build-info.json": stableJson({
      demo,
      development,
      base: location.base,
      siteBase: location.siteBase,
      repository: location.repository,
      pages,
    }),
    ".nojekyll": "",
  };
  for (const [path, bytes] of Object.entries(publicFiles)) await write(path, bytes);
  await copyFile(join(root, "src/blog/mark.svg"), join(output, "mark.svg"));
  await copyFile(join(root, "src/blog/favicon.png"), join(output, "favicon.png"));
  return { output, entries, publicFiles };
}

export async function blogBuild({
  root,
  base,
  origin,
  development,
  demo,
  repository = process.env.SITE_REPOSITORY || process.env.GITHUB_REPOSITORY,
}) {
  root = await realpath(root);
  const generated = await generate({ root, base, origin, development, demo, repository });
  const readFileMark = await readFile(join(root, "src/blog/mark.svg"));
  const readFileFavicon = await readFile(join(root, "src/blog/favicon.png"));
  return {
    entries: generated.entries,
    plugin: {
      name: "tmt-blog",
      enforce: "pre",
      // Config loading (lint, format, tests) also generates the pages, so the origin is demanded
      // where a build starts: a production artifact never carries the neutral local origin.
      buildStart() {
        if (!demo && !development) repositoryUrl(repository);
        if (!origin && !demo && !development)
          throw new Error(
            "SITE_ORIGIN is required for a production build, for example https://example.com",
          );
      },
      // Article modules are self-contained. Do not let a reviewed snippet pull
      // unapproved files or packages into its build from elsewhere in the repo.
      resolveId(source, importer) {
        if (importer?.startsWith(generated.output + "/") && importer.endsWith(".mjs"))
          throw new Error(`Article modules cannot import dependencies: ${source}`);
        return null;
      },
      generateBundle() {
        for (const [path, source] of Object.entries(generated.publicFiles))
          this.emitFile({ type: "asset", fileName: `blog/${path}`, source });
        // Layout URLs already include SITE_BASE. vite-ignore leaves them unchanged in dev
        // and build; this plugin owns the file instead of Vite inlining or rebasing it.
        this.emitFile({ type: "asset", fileName: "blog/mark.svg", source: readFileMark });
        this.emitFile({ type: "asset", fileName: "blog/favicon.png", source: readFileFavicon });
      },
    },
  };
}
