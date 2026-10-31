import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import { metadata, validateApproval, validateStoredArticle } from "./content.mjs";
import { languages, languageInfo, indexPath, postPath } from "./languages.mjs";
import { tagPath } from "./tags.mjs";

export const escapeHtml = (value) =>
  String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
export const csp =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

// Where a local run (dev server, demo, unit test) claims to live. A production build names its own
// origin (SITE_ORIGIN) and never falls back to this.
export const localOrigin = "https://localhost";

export function assertSiteOrigin(origin) {
  const url = URL.canParse(origin) ? new URL(origin) : null;
  if (url?.origin !== origin || url.protocol !== "https:")
    throw new Error("SITE_ORIGIN must be an HTTPS origin without a trailing slash or path");
}

export function repositoryUrl(
  repository = process.env.SITE_REPOSITORY || process.env.GITHUB_REPOSITORY,
) {
  if (
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? "") ||
    repository.split("/").some((part) => part === "." || part === "..")
  )
    throw new Error("SITE_REPOSITORY or GITHUB_REPOSITORY must name owner/repository");
  return `https://github.com/${repository}`;
}

export function locationConfig({ base, origin = localOrigin, repository } = {}) {
  if (!/^\/(?:[A-Za-z0-9_-]+\/)*$/.test(base))
    throw new Error("SITE_BASE must be an absolute directory path ending in /");
  assertSiteOrigin(origin);
  return {
    siteBase: base,
    base: base + "blog/",
    origin,
    url: origin + base + "blog/",
    repository: repositoryUrl(
      repository ||
        process.env.SITE_REPOSITORY ||
        process.env.GITHUB_REPOSITORY ||
        "example/repository",
    ),
  };
}

export async function filesBelow(root, prefix = "") {
  const result = {};
  for (const item of await readdir(root, { withFileTypes: true })) {
    const relative = prefix + item.name;
    const path = join(root, item.name);
    if (item.isSymbolicLink()) throw new Error(`Symlink not allowed: ${relative}`);
    if (item.isDirectory()) Object.assign(result, await filesBelow(path, relative + "/"));
    else if (item.isFile()) result[relative] = await readFile(path);
    else throw new Error(`Unsupported file: ${relative}`);
  }
  return result;
}

export async function readPosts(directory) {
  const posts = [];
  for (const item of await readdir(directory, { withFileTypes: true })) {
    if (item.name === ".gitkeep") continue;
    if (!item.isDirectory() || item.isSymbolicLink())
      throw new Error(`Unexpected post entry: ${item.name}`);
    const allFiles = await filesBelow(join(directory, item.name));
    for (const language of languages) {
      const prefix = language.id === "en" ? "" : language.id + "/";
      const files = Object.fromEntries(
        Object.entries(allFiles)
          .filter(([path]) =>
            prefix
              ? path.startsWith(prefix)
              : !languages.some(({ id }) => id !== "en" && path.startsWith(id + "/")),
          )
          .map(([path, bytes]) => [path.slice(prefix.length), bytes]),
      );
      if (prefix && !Object.keys(files).length) continue;
      for (const required of ["meta.json", "article.html", "approval.json"])
        if (!files[required]) throw new Error(`${item.name}: missing ${required}`);
      const meta = metadata(JSON.parse(files["meta.json"]));
      if (meta.slug !== item.name) throw new Error("Post directory must match slug");
      if (meta.language !== language.id) throw new Error("Post directory must match language");
      const html = files["article.html"].toString("utf8");
      const approval = JSON.parse(files["approval.json"]);
      const assets = Object.fromEntries(
        Object.entries(files).filter(
          ([path]) => !["meta.json", "article.html", "approval.json"].includes(path),
        ),
      );
      validateStoredArticle(html, assets);
      validateApproval(approval, meta, html, assets);
      posts.push({ meta, html, assets });
    }
  }
  return posts.sort(
    (a, b) => b.meta.date.localeCompare(a.meta.date) || a.meta.slug.localeCompare(b.meta.slug),
  );
}

export function layout({
  site,
  location,
  title,
  description,
  path = "",
  language = "en",
  body,
  demo = false,
  development = false,
  requestedLanguage = "en",
  alternates = [],
  canonicalPath = path,
  fallback = false,
  slug,
  tag,
}) {
  const e = escapeHtml;
  const policy = development
    ? csp.replace("connect-src 'none'", "connect-src 'self' ws://127.0.0.1:*")
    : csp;
  const languageIndex = location.base + indexPath(requestedLanguage);
  const alternateLinks = alternates
    .map(
      ({ language, path }) =>
        `<link rel="alternate" hreflang="${e(languageInfo(language).tag)}" href="${e(location.url + path)}">`,
    )
    .join("");
  const canonical = `<link rel="canonical" href="${e(location.url + canonicalPath)}">`;
  const englishAlternate = alternates.find((item) => item.language === "en");
  const defaultAlternate = englishAlternate
    ? `<link rel="alternate" hreflang="x-default" href="${e(location.url + englishAlternate.path)}">`
    : "";
  const switcher = languages
    .map(({ id, tag: htmlTag, label }) => {
      const available = alternates.some((item) => item.language === id);
      if (tag && !available)
        return `<span aria-disabled="true" title="No ${e(label)} articles for #${e(tag)}">${e(id === "en" ? "EN" : id === "ja" ? "JA" : languageInfo(id).tag)}</span>`;
      const name = label + (available ? "" : " (English fallback)");
      const url =
        location.base + (slug ? postPath(slug, id) : tag ? tagPath(tag, id) : indexPath(id));
      return `<a href="${e(url)}"${id === requestedLanguage ? ' aria-current="page"' : ""} aria-label="${e(name)}" title="${e(name)}">${e(id === "en" ? "EN" : id === "ja" ? "JA" : htmlTag)}</a>`;
    })
    .join("");
  return `<!doctype html>
<html lang="${e(language)}"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="${e(policy)}">
<title>${e(title)}</title><meta name="description" content="${e(description)}">
${demo ? '<meta name="robots" content="noindex,nofollow">' : `${fallback ? '<meta name="robots" content="noindex,follow">' : ""}${canonical}${alternateLinks}${defaultAlternate}`}
<link vite-ignore rel="alternate" type="application/rss+xml" title="${e(site.title)}" href="${e(languageIndex)}feed.xml">
<link vite-ignore rel="icon" href="${e(location.base)}favicon.png" type="image/png">
<link vite-ignore rel="apple-touch-icon" href="${e(location.base)}favicon.png">
<link rel="stylesheet" href="/src/blog/blog.css">
</head><body>
<a class="skip-link" lang="en" href="#main">Skip to content</a>
<header class="site-header" lang="en"><a class="brand" href="${e(languageIndex)}"><img vite-ignore src="${e(location.base)}mark.svg" width="30" height="30" alt=""><span>tmt</span><span class="brand-divider"></span><span>journal</span></a><nav aria-label="Main"><a href="${e(languageIndex)}">Articles</a><a href="${e(languageIndex)}feed.xml">RSS ↗</a><a href="${e(location.repository)}">Explore TMT ↗</a></nav></header>
<nav class="language-switcher" aria-label="${slug ? "Article" : tag ? "Topic" : "Index"} language" lang="en">${switcher}</nav>
${demo ? '<div class="preview-notice">◇ Template preview · Example content, not a published post</div>' : ""}
<main id="main">${body}</main>
<footer class="site-footer" lang="en"><span>TMT Journal</span><span>Tools for people and agents.</span><a href="${e(location.repository)}">TMT ↗</a></footer>
</body></html>\n`;
}

function tagLinks(meta, location) {
  if (!meta.tags?.length) return "";
  return `<ul class="post-tags" aria-label="Tags" lang="en">${meta.tags.map((tag) => `<li><a href="${escapeHtml(location.base + tagPath(tag, meta.language))}">#${escapeHtml(tag)}</a></li>`).join("")}</ul>`;
}

export function indexBody(posts, site, location, language = "en", fallback = false, { tag } = {}) {
  const e = escapeHtml;
  return `<section class="index-intro" lang="en"><p class="eyebrow">${tag ? "TOPIC" : "THE TMT JOURNAL"}</p><h1${tag ? ' class="tag-title"' : ""}>${tag ? "#" + e(tag) : "Field notes.<br>Working ideas."}</h1><p class="intro-description">${tag ? "Articles tagged #" + e(tag) + "." : e(site.description)}</p>${fallback ? `<p class="translation-notice">${e(languageInfo(language).label)} translations are not available yet. Showing the English index.</p>` : ""}</section>
<section class="article-index" aria-labelledby="articles-title"><div class="section-heading" lang="en"><h2 id="articles-title">${tag ? "Tagged articles" : "All articles"}</h2><span>${String(posts.length).padStart(2, "0")} ARTICLES</span></div>
${posts.length ? posts.map(({ meta }, index) => `<div class="article-row" lang="${languageInfo(meta.language).tag}"><span class="article-number">${String(index + 1).padStart(2, "0")}</span><div><time datetime="${meta.date}">${meta.date}</time><h3><a href="${e(location.base + postPath(meta.slug, language))}">${e(meta.title)}</a></h3><p>${e(meta.summary)}</p>${tagLinks(meta, location)}${meta.language !== language ? '<p class="fallback-label" lang="en">English · translation not available yet</p>' : ""}</div><span class="article-arrow" aria-hidden="true">↗</span></div>`).join("\n") : '<div class="empty-state" lang="en"><p class="eyebrow">A SPACE FOR WHAT COMES NEXT</p><h3>The first note is on its way.</h3><p>New articles will appear here when they are ready.</p></div>'}
</section>`;
}

export function postBody(post, location, language = "en") {
  const e = escapeHtml;
  const { meta } = post;
  return `<div class="post-shell"><a class="back-link" lang="en" href="${e(location.base + indexPath(language))}">← All articles</a>${meta.language !== language ? `<p class="translation-notice" lang="en">${e(languageInfo(language).label)} translation is not available yet. Showing the English article.</p>` : ""}<header class="post-heading"><p class="eyebrow">TMT JOURNAL <span> / </span> <time datetime="${meta.date}">${meta.date}</time></p><h1>${e(meta.title)}</h1><p class="post-summary">${e(meta.summary)}</p>${tagLinks(meta, location)}</header>${post.html}</div>`;
}

export function rss(posts, site, location, language = "en") {
  const e = escapeHtml;
  const channelPath = posts.length ? indexPath(language) : "";
  return `<?xml version="1.0" encoding="UTF-8"?>\n<rss version="2.0"><channel><title>${e(site.title)}</title><link>${e(location.url + channelPath)}</link><language>${languageInfo(language).tag}</language><description>${e(site.description)}</description>${posts.map(({ meta }) => `<item><title>${e(meta.title)}</title><link>${e(location.url + postPath(meta.slug, language))}</link><guid isPermaLink="true">${e(location.url + postPath(meta.slug, language))}</guid><description>${e(meta.summary)}</description>${(meta.tags ?? []).map((tag) => `<category>${e(tag)}</category>`).join("")}<pubDate>${new Date(meta.date + "T00:00:00Z").toUTCString()}</pubDate></item>`).join("")}</channel></rss>\n`;
}
