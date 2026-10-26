import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "parse5";
import { SaxesParser } from "saxes";
import { attr, walk } from "./content.mjs";
import { assertSiteOrigin, filesBelow, csp } from "./site.mjs";
import { languageInfo } from "./languages.mjs";

// A production artifact is checked against the origin it was built for (SITE_ORIGIN): its canonical
// links, RSS and sitemap must name it. A demo artifact has no canonical links and an empty feed.
export async function verifyDist(
  root = resolve("dist"),
  { demo = false, blogDirectory = "blog", origin } = {},
) {
  const allFiles = await filesBelow(root);
  const prefix = blogDirectory ? blogDirectory + "/" : "";
  const files = Object.fromEntries(
    Object.entries(allFiles)
      .filter(([path]) => path.startsWith(prefix))
      .map(([path, bytes]) => [path.slice(prefix.length), bytes]),
  );
  const info = JSON.parse(files["build-info.json"]);
  if (info.development) throw new Error("Development policy must never be deployed");
  if (info.demo !== demo)
    throw new Error(
      demo
        ? "Expected an explicit demo artifact"
        : "Expected a production artifact; demo output requires --demo",
    );
  if (!demo) {
    if (!origin) throw new Error("SITE_ORIGIN is required to verify a production artifact");
    assertSiteOrigin(origin);
  }
  for (const path of Object.keys(allFiles))
    if (
      /(?:^|\/)(?:\.private|conversations(?:\.(?:json|md))?|manifest\.json|approval\.json|meta\.json|source\.html)(?:\/|$)/.test(
        path,
      )
    )
      throw new Error(`Private source in dist: ${path}`);
  const htmlPaths = Object.keys(files).filter((path) => path.endsWith(".html"));
  if (!Array.isArray(info.pages) || !info.pages.length) throw new Error("Missing page inventory");
  const expectedPages = new Map(info.pages.map((page) => [page.path + "index.html", page]));
  if (expectedPages.size !== info.pages.length || htmlPaths.length !== expectedPages.size)
    throw new Error("Article HTML count mismatch");
  if (!files["index.html"] || !files["feed.xml"] || !files["sitemap.xml"] || !files[".nojekyll"]) {
    // An empty .nojekyll Buffer is truthy; file existence, not byte count, matters.
    throw new Error("Missing static site outputs");
  }
  const siteBase = info.siteBase ?? info.base;
  const checkLink = (raw, owner, image = false) => {
    if (/^data:/i.test(raw ?? "")) {
      if (image && /^data:image\//i.test(raw)) return;
      throw new Error(`Data URL is not an image source: ${raw}`);
    }
    if (!raw || /^(?:https?:|mailto:|#)/.test(raw)) return;
    const url = new URL(raw, `https://build.invalid${info.base}${owner}`);
    if (url.origin !== "https://build.invalid" || !url.pathname.startsWith(siteBase))
      throw new Error(`Link escapes SITE_BASE: ${raw}`);
    const artifactBase = info.siteBase ? info.siteBase : info.base;
    let path = decodeURIComponent(url.pathname.slice(artifactBase.length));
    if (!path || path.endsWith("/")) path += "index.html";
    if (!allFiles[path]) throw new Error(`Missing target ${raw} from ${owner}`);
  };
  for (const path of htmlPaths) {
    const page = expectedPages.get(path);
    if (!page) throw new Error(`Unexpected HTML page: ${path}`);
    const route = page.path.match(
      /^(?:(ja|zh-hant|zh-hans)\/)?(?:posts\/([a-z0-9]+(?:-[a-z0-9]+)*)\/)?$/,
    );
    if (!route) throw new Error(`Invalid blog route: ${path}`);
    if (page.fallbackTo && (!route[1] || !route[2] || page.fallbackTo !== `posts/${route[2]}/`))
      throw new Error(`Invalid English fallback target: ${path}`);
    const html = files[path].toString("utf8");
    let policy = false;
    let article = null;
    let canonical = null;
    let htmlLanguage = null;
    const alternates = [];
    walk(parse(html), (node) => {
      if (node.tagName === "html") htmlLanguage = attr(node, "lang");
      if (node.tagName === "article" && attr(node, "id") === "blog-article") article = node;
      if (node.tagName === "link" && attr(node, "rel") === "canonical")
        canonical = attr(node, "href");
      if (node.tagName === "link" && attr(node, "hreflang"))
        alternates.push({ href: attr(node, "href"), tag: attr(node, "hreflang") });
      if (node.attrs?.some(({ name }) => /^on/i.test(name)))
        throw new Error(`Inline event handler in ${path}`);
      if (
        node.tagName === "meta" &&
        attr(node, "http-equiv") === "Content-Security-Policy" &&
        attr(node, "content") === csp
      )
        policy = true;
      if (node.tagName === "script" && (!attr(node, "src") || node.childNodes.length))
        throw new Error(`Inline script in ${path}`);
      if (node.tagName === "iframe" || node.tagName?.startsWith("tmt-"))
        throw new Error(`Private/embedded chrome in ${path}`);
      if (node.tagName === "script" && /^(?:https?:)?\/\//.test(attr(node, "src")))
        throw new Error("External script origin");
      if (node.tagName === "script" && /^[a-z]+:/i.test(attr(node, "src")))
        throw new Error("Non-local script URL");
      if (node.tagName === "img" && /^(?:https?:)?\/\//.test(attr(node, "src")))
        throw new Error("External image origin");
      for (const name of ["href", "src"])
        checkLink(attr(node, name), path, name === "src" && node.tagName === "img");
    });
    if (info.demo && !html.includes('name="robots" content="noindex,nofollow"'))
      throw new Error(`Demo page must be noindex: ${path}`);
    if (!policy || !html.includes("<h1"))
      throw new Error(`Missing CSP or rendered content in ${path}`);
    if (!demo) {
      const expected = origin + info.base + (page.fallbackTo ?? page.path);
      if (canonical !== expected)
        throw new Error(`Canonical link of ${path} must be ${expected}, found ${canonical}`);
    }
    for (const { href, tag } of alternates) {
      const url = URL.canParse(href) ? new URL(href) : null;
      const target = url?.pathname.startsWith(info.base)
        ? expectedPages.get(url.pathname.slice(info.base.length) + "index.html")
        : null;
      if (
        !target ||
        target.fallbackTo ||
        url.origin !== origin ||
        (tag === "x-default" ? target.language !== "en" : tag !== target.language)
      )
        throw new Error(`Invalid language alternate in ${path}: ${href}`);
    }
    if (
      page.language !== languageInfo(page.fallbackTo ? "en" : (route[1] ?? "en")).tag ||
      htmlLanguage !== page.language
    )
      throw new Error(`Wrong HTML language in ${path}`);
    if (
      page.fallbackTo &&
      (!expectedPages.has(page.fallbackTo + "index.html") ||
        (!demo && !html.includes('name="robots" content="noindex,follow"')))
    )
      throw new Error(`Invalid or indexable fallback: ${path}`);
    if (/^(?:(?:ja|zh-hant|zh-hans)\/)?posts\//.test(path)) {
      let visibleText = "";
      const text = (node) => {
        if (["style", "script"].includes(node.tagName)) return;
        if (node.nodeName === "#text") visibleText += node.value;
        for (const child of node.childNodes ?? []) text(child);
      };
      if (article) text(article);
      if (!visibleText.trim()) throw new Error("Article is an empty SPA shell");
    }
  }
  const feed = files["feed.xml"].toString("utf8");
  for (const path of Object.keys(files).filter(
    (path) => path.endsWith("feed.xml") || path === "sitemap.xml",
  )) {
    const xml = new SaxesParser({ xmlns: true });
    let element = "";
    xml.on("opentag", (tag) => {
      element = tag.local;
    });
    xml.on("error", (error) => {
      throw error;
    });
    xml.on("text", (value) => {
      if (["link", "guid", "loc"].includes(element) && value.startsWith("https://")) {
        const parsed = new URL(value);
        if (!demo && parsed.origin !== origin)
          throw new Error(`${path} names ${parsed.origin}, not the site origin ${origin}`);
        const local = info.base + parsed.pathname.slice(info.base.length);
        if (!parsed.pathname.startsWith(info.base))
          throw new Error("Feed/sitemap URL escapes base");
        checkLink(local, "index.html");
        const linked = expectedPages.get(parsed.pathname.slice(info.base.length) + "index.html");
        if (linked?.fallbackTo) throw new Error(`Fallback URL in feed/sitemap: ${value}`);
      }
    });
    xml.on("closetag", () => {
      element = "";
    });
    xml.write(files[path].toString("utf8")).close();
    if (info.demo && path.endsWith("feed.xml") && files[path].toString("utf8").includes("<item>"))
      throw new Error("Examples must not enter RSS or indexing");
  }
  if (!feed.startsWith("<?xml") || !feed.includes('<rss version="2.0">'))
    throw new Error("Invalid RSS document");
  if (
    info.demo &&
    (feed.includes("<item>") || !files["robots.txt"].toString().includes("Disallow: /"))
  )
    throw new Error("Examples must not enter RSS or indexing");
  for (const path of expectedPages.keys())
    if (!files[path]) throw new Error("Missing prerendered article");
  console.log(`Verified ${htmlPaths.length} rendered HTML pages, local links, CSP and RSS.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.some((arg) => arg !== "--demo")) throw new Error("Usage: verify-dist.mjs [--demo]");
  await verifyDist(undefined, {
    demo: args.includes("--demo"),
    origin: process.env.SITE_ORIGIN || undefined,
  });
}
