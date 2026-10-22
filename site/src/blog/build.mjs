import { mkdir, readFile, writeFile, rm, copyFile, realpath } from "node:fs/promises";
import { dirname, join } from "node:path";
import { extractArticle, articleFromSource, metadata, stableJson } from "./content.mjs";
import { indexBody, layout, locationConfig, postBody, readPosts, rss } from "./site.mjs";

// Generated HTML lives under the Vite root so its output paths are blog/**.
// Only this directory is disposable; approved src/posts and review records are not.
export async function generate({ root, demo = false, development = false, base, origin }) {
  const location = locationConfig({ base, origin });
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
  const entries = [join(output, "index.html")];
  await write(
    "index.html",
    layout({
      site,
      location,
      title: site.title,
      description: site.description,
      demo,
      development,
      body: indexBody(posts, site, location),
    }),
  );
  for (const post of posts) {
    const path = `posts/${post.meta.slug}/`;
    entries.push(join(output, path, "index.html"));
    await write(
      path + "index.html",
      layout({
        site,
        location,
        title: `${post.meta.title} · ${site.title}`,
        description: post.meta.summary,
        path,
        language: post.meta.language,
        demo,
        development,
        body: postBody(post, location),
      }),
    );
    for (const [asset, bytes] of Object.entries(post.assets)) await write(path + asset, bytes);
  }
  const publicFiles = {
    "feed.xml": rss(demo ? [] : posts, site, location),
    "robots.txt": demo
      ? "User-agent: *\nDisallow: /\n"
      : `User-agent: *\nAllow: ${location.base}\nSitemap: ${location.url}sitemap.xml\n`,
    "sitemap.xml": `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${(demo ? [] : ["", ...posts.map(({ meta }) => `posts/${meta.slug}/`)]).map((path) => `<url><loc>${location.url}${path}</loc></url>`).join("")}</urlset>\n`,
    "build-info.json": stableJson({
      demo,
      development,
      base: location.base,
      siteBase: location.siteBase,
      articles: posts.map(({ meta }) => meta.slug),
    }),
    ".nojekyll": "",
  };
  for (const [path, bytes] of Object.entries(publicFiles)) await write(path, bytes);
  await copyFile(join(root, "src/blog/mark.svg"), join(output, "mark.svg"));
  return { output, entries, publicFiles };
}

export async function blogBuild({ root, base, origin, development, demo }) {
  root = await realpath(root);
  const generated = await generate({ root, base, origin, development, demo });
  const readFileMark = await readFile(join(root, "src/blog/mark.svg"));
  return {
    entries: generated.entries,
    plugin: {
      name: "tmt-blog",
      enforce: "pre",
      // The mark is the blog's tab icon and header image; keep it a file, never an inline data URL.
      config: () => ({
        build: {
          assetsInlineLimit: (file) => (file.endsWith("/blog/mark.svg") ? false : undefined),
        },
      }),
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
        this.emitFile({ type: "asset", fileName: "blog/mark.svg", source: readFileMark });
      },
    },
  };
}
