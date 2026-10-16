import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import { metadata, validateApproval, validateStoredArticle } from "./content.mjs";

export const escapeHtml = (value) =>
  String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
export const csp =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

export function locationConfig({ base, origin = "https://pj-tmt.github.io" } = {}) {
  if (!/^\/(?:[A-Za-z0-9_-]+\/)*$/.test(base))
    throw new Error("SITE_BASE must be an absolute directory path ending in /");
  const url = new URL(origin);
  if (url.origin !== origin || url.protocol !== "https:")
    throw new Error("SITE_ORIGIN must be an HTTPS origin without a trailing slash or path");
  return { siteBase: base, base: base + "blog/", origin, url: origin + base + "blog/" };
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
    const files = await filesBelow(join(directory, item.name));
    for (const required of ["meta.json", "article.html", "approval.json"])
      if (!files[required]) throw new Error(`${item.name}: missing ${required}`);
    const meta = metadata(JSON.parse(files["meta.json"]));
    if (meta.slug !== item.name) throw new Error("Post directory must match slug");
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
}) {
  const e = escapeHtml;
  const policy = development
    ? csp.replace("connect-src 'none'", "connect-src 'self' ws://127.0.0.1:*")
    : csp;
  return `<!doctype html>
<html lang="${e(language)}"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="${e(policy)}">
<title>${e(title)}</title><meta name="description" content="${e(description)}">
${demo ? '<meta name="robots" content="noindex,nofollow">' : `<link rel="canonical" href="${e(location.url + path)}">`}
<link vite-ignore rel="alternate" type="application/rss+xml" title="${e(site.title)}" href="${e(location.base)}feed.xml">
<link rel="icon" href="${e(location.base)}mark.svg" type="image/svg+xml">
<link rel="stylesheet" href="/src/blog/blog.css">
</head><body>
<a class="skip-link" href="#main">Skip to content</a>
<header class="site-header"><a class="brand" href="${e(location.base)}"><img src="${e(location.base)}mark.svg" width="24" height="24" alt=""><span>TMT</span><span class="brand-divider"></span><span>Journal</span></a><nav aria-label="Main"><a href="${e(location.base)}">Articles</a><a href="${e(location.base)}feed.xml">RSS ↗</a><a href="${e(location.siteBase)}">Explore TMT ↗</a></nav></header>
${demo ? '<div class="preview-notice">◇ Template preview · Example content, not a published post</div>' : ""}
<main id="main">${body}</main>
<footer class="site-footer"><span>TMT Journal</span><span>Tools for people and agents.</span><a href="${e(location.siteBase)}">TMT ↗</a></footer>
</body></html>\n`;
}

export function indexBody(posts, site, location) {
  const e = escapeHtml;
  return `<section class="index-intro"><p class="eyebrow">THE TMT JOURNAL</p><h1>Field notes.<br>Working ideas.</h1><p class="intro-description">${e(site.description)}</p></section>
<section class="article-index" aria-labelledby="articles-title"><div class="section-heading"><h2 id="articles-title">All articles</h2><span>${String(posts.length).padStart(2, "0")} ARTICLES</span></div>
${posts.length ? posts.map(({ meta }, index) => `<a class="article-row" href="${e(location.base)}posts/${meta.slug}/"><span class="article-number">${String(index + 1).padStart(2, "0")}</span><div><time datetime="${meta.date}">${meta.date}</time><h3>${e(meta.title)}</h3><p>${e(meta.summary)}</p></div><span class="article-arrow" aria-hidden="true">↗</span></a>`).join("\n") : '<div class="empty-state"><p class="eyebrow">A SPACE FOR WHAT COMES NEXT</p><h3>The first note is on its way.</h3><p>New articles will appear here when they are ready.</p></div>'}
</section>`;
}

export function postBody(post, location) {
  const e = escapeHtml;
  const { meta } = post;
  return `<div class="post-shell"><a class="back-link" href="${e(location.base)}">← All articles</a><header class="post-heading"><p class="eyebrow">TMT JOURNAL <span> / </span> <time datetime="${meta.date}">${meta.date}</time></p><h1>${e(meta.title)}</h1><p>${e(meta.summary)}</p></header>${post.html}</div>`;
}

export function rss(posts, site, location) {
  const e = escapeHtml;
  return `<?xml version="1.0" encoding="UTF-8"?>\n<rss version="2.0"><channel><title>${e(site.title)}</title><link>${e(location.url)}</link><description>${e(site.description)}</description>${posts.map(({ meta }) => `<item><title>${e(meta.title)}</title><link>${e(location.url)}posts/${meta.slug}/</link><guid isPermaLink="true">${e(location.url)}posts/${meta.slug}/</guid><description>${e(meta.summary)}</description><pubDate>${new Date(meta.date + "T00:00:00Z").toUTCString()}</pubDate></item>`).join("")}</channel></rss>\n`;
}
