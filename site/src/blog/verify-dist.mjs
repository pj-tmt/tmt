import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "parse5";
import { SaxesParser } from "saxes";
import { attr, walk } from "./content.mjs";
import { filesBelow, csp } from "./site.mjs";

export async function verifyDist(
  root = resolve("dist"),
  { demo = false, blogDirectory = "blog" } = {},
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
  for (const path of Object.keys(allFiles))
    if (
      /(?:^|\/)(?:\.private|conversations(?:\.(?:json|md))?|manifest\.json|approval\.json|meta\.json|source\.html)(?:\/|$)/.test(
        path,
      )
    )
      throw new Error(`Private source in dist: ${path}`);
  const htmlPaths = Object.keys(files).filter((path) => path.endsWith(".html"));
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
    const html = files[path].toString("utf8");
    let policy = false;
    let article = null;
    walk(parse(html), (node) => {
      if (node.tagName === "article" && attr(node, "id") === "blog-article") article = node;
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
    if (path.startsWith("posts/")) {
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
  for (const path of ["feed.xml", "sitemap.xml"]) {
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
        const local = info.base + parsed.pathname.slice(info.base.length);
        if (!parsed.pathname.startsWith(info.base))
          throw new Error("Feed/sitemap URL escapes base");
        checkLink(local, "index.html");
      }
    });
    xml.on("closetag", () => {
      element = "";
    });
    xml.write(files[path].toString("utf8")).close();
  }
  if (!feed.startsWith("<?xml") || !feed.includes('<rss version="2.0">'))
    throw new Error("Invalid RSS document");
  if (
    info.demo &&
    (feed.includes("<item>") || !files["robots.txt"].toString().includes("Disallow: /"))
  )
    throw new Error("Examples must not enter RSS or indexing");
  if (htmlPaths.length !== info.articles.length + 1) throw new Error("Article HTML count mismatch");
  for (const slug of info.articles)
    if (!files[`posts/${slug}/index.html`]) throw new Error("Missing prerendered article");
  console.log(`Verified ${htmlPaths.length} rendered HTML pages, local links, CSP and RSS.`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.some((arg) => arg !== "--demo")) throw new Error("Usage: verify-dist.mjs [--demo]");
  await verifyDist(undefined, { demo: args.includes("--demo") });
}
