import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "parse5";
import { parseAst } from "vite-plus";
import { attr, walk } from "./content.mjs";
import { csp, escapeHtml, filesBelow, repositoryUrl } from "./site.mjs";

const buildOrigin = "https://build.invalid";

// Canonicals and navigation may be external. Resource dependencies must be local
// (or an embedded data image), so a Pages artifact cannot hide missing resources.
function target(raw, owner, info, resource = false) {
  if (!raw || raw.startsWith("#")) return null;
  if (resource && /^data:image\//i.test(raw)) return null;
  const url = new URL(raw, `${buildOrigin}${info.siteBase}${owner}`);
  if (![buildOrigin, info.origin].includes(url.origin)) {
    if (resource) throw new Error(`External resource from ${owner}: ${raw}`);
    return null;
  }
  if (!url.pathname.startsWith(info.siteBase))
    throw new Error(`Link escapes SITE_BASE from ${owner}: ${raw}`);
  let path = decodeURIComponent(url.pathname.slice(info.siteBase.length));
  if (!path || path.endsWith("/")) path += "index.html";
  return path;
}

function references(path, bytes, info) {
  const found = [];
  const add = (raw, resource = true) => {
    const dependency = target(raw, path, info, resource);
    if (dependency) found.push(dependency);
  };
  const css = (source) => {
    for (const match of source.matchAll(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^\s)]*))\s*\)/gi))
      add(match[1] ?? match[2] ?? match[3]);
    for (const match of source.matchAll(/@import\s+["']([^"']+)["']/gi)) add(match[1]);
  };
  const source = bytes.toString("utf8");
  if (path.endsWith(".html")) {
    walk(parse(source), (node) => {
      for (const name of ["href", "src", "poster"])
        if (attr(node, name)) add(attr(node, name), name !== "href" || node.tagName === "link");
      if (node.tagName === "style")
        css((node.childNodes ?? []).map((child) => child.value ?? "").join(""));
      if (attr(node, "style")) css(attr(node, "style"));
    });
  } else if (path.endsWith(".css")) css(source);
  else if (/\.m?js$/.test(path)) {
    const visit = (node) => {
      if (!node || typeof node !== "object") return;
      if (
        [
          "ImportDeclaration",
          "ExportNamedDeclaration",
          "ExportAllDeclaration",
          "ImportExpression",
        ].includes(node.type) &&
        node.source
      ) {
        if (typeof node.source.value !== "string")
          throw new Error(`Non-static module dependency in ${path}`);
        add(node.source.value);
      }
      if (
        node.type === "NewExpression" &&
        node.callee?.name === "URL" &&
        node.arguments[1]?.object?.type === "MetaProperty" &&
        node.arguments[1]?.property?.name === "url"
      ) {
        if (typeof node.arguments[0]?.value !== "string")
          throw new Error(`Non-static asset URL in ${path}`);
        add(node.arguments[0].value);
      }
      for (const value of Object.values(node)) {
        if (Array.isArray(value)) value.forEach(visit);
        else if (value && typeof value === "object") visit(value);
      }
    };
    visit(parseAst(source));
  }
  return found;
}

// Keep all generated blog routes/data and only their reachable shared assets.
// Home, locale Homes, chapter redirects and Colab never become traversal roots.
function reachableFiles(files, info) {
  const kept = new Set(Object.keys(files).filter((path) => path.startsWith("blog/")));
  kept.add("index.html");
  kept.add("404.html");
  for (const path of kept) {
    if (!files[path]) throw new Error(`Missing Pages file: ${path}`);
    for (const dependency of references(path, files[path], info)) {
      if (!files[dependency]) throw new Error(`Missing Pages target ${dependency} from ${path}`);
      if (
        !dependency.startsWith("blog/") &&
        !dependency.startsWith("assets/") &&
        !["index.html", "404.html"].includes(dependency)
      )
        throw new Error(`Non-blog Pages target ${dependency} from ${path}`);
      kept.add(dependency);
    }
  }
  return kept;
}

function entryPage(info, missing = false) {
  const blog = escapeHtml(info.base);
  const repository = escapeHtml(info.repository);
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="${escapeHtml(csp)}">
<meta name="robots" content="noindex,follow">
${missing ? "" : `<meta http-equiv="refresh" content="0;url=${blog}">`}
<title>${missing ? "Page not found" : "TMT Journal"}</title></head>
<body><h1>${missing ? "Page not found" : "TMT Journal"}</h1>
<p><a href="${blog}">${missing ? "Visit" : "Continue to"} the TMT Journal</a></p>
${missing ? `<p><a href="${repository}">TMT on GitHub</a></p>` : ""}
</body></html>\n`;
}

export function verifyPages(files, info, { origin, repository } = {}) {
  if (info.siteBase + "blog/" !== info.base) throw new Error("Invalid Pages site base");
  const expectedRepository = repositoryUrl(repository);
  if (info.repository !== expectedRepository)
    throw new Error(`Pages repository must be ${expectedRepository}`);
  for (const path of ["index.html", "404.html"]) {
    if (!files[path]) throw new Error(`Missing Pages file: ${path}`);
    const html = parse(files[path].toString("utf8"));
    const links = [];
    let refresh;
    walk(html, (node) => {
      if (node.tagName === "a") links.push(attr(node, "href"));
      if (node.tagName === "meta" && attr(node, "http-equiv")?.toLowerCase() === "refresh")
        refresh = attr(node, "content");
      if (node.tagName === "script" || attr(node, "id") === "root")
        throw new Error(`Pages entry must be static: ${path}`);
    });
    if (
      !links.includes(info.base) ||
      (path === "index.html"
        ? refresh !== `0;url=${info.base}`
        : !links.includes(expectedRepository))
    )
      throw new Error(`Invalid Pages ${path === "index.html" ? "root redirect" : "404 links"}`);
  }
  for (const [path, bytes] of Object.entries(files)) {
    if (!path.startsWith("blog/") || !path.endsWith(".html")) continue;
    const shellLinks = [];
    walk(parse(bytes.toString("utf8")), (node) => {
      if (!["site-header", "site-footer"].includes(attr(node, "class"))) return;
      const outbound = [];
      walk(node, (child) => {
        if (child.tagName === "a" && /^https?:/.test(attr(child, "href") ?? ""))
          outbound.push(attr(child, "href"));
      });
      shellLinks.push(outbound);
    });
    if (
      shellLinks.length !== 2 ||
      shellLinks.some((links) => links.length !== 1 || links[0] !== expectedRepository)
    )
      throw new Error(`Blog header/footer must link to the repository: ${path}`);
  }
  const kept = reachableFiles(files, { ...info, origin });
  for (const path of Object.keys(files)) {
    if (
      path.endsWith(".html") &&
      !path.startsWith("blog/") &&
      !["index.html", "404.html"].includes(path)
    )
      throw new Error(`Non-blog HTML in Pages: ${path}`);
    if (!kept.has(path)) throw new Error(`Unreferenced file in Pages: ${path}`);
  }
}

export async function prunePages(
  source = resolve("dist"),
  output = resolve("dist-pages"),
  {
    origin = process.env.SITE_ORIGIN,
    repository = process.env.SITE_REPOSITORY || process.env.GITHUB_REPOSITORY,
  } = {},
) {
  source = resolve(source);
  output = resolve(output);
  if (source === output || source.startsWith(output + sep) || output.startsWith(source + sep))
    throw new Error("Pages output must be separate from the full build");
  const files = await filesBelow(source);
  const info = JSON.parse(files["blog/build-info.json"]);
  if (info.demo || info.development) throw new Error("Pages requires a production blog build");
  files["index.html"] = Buffer.from(entryPage(info));
  files["404.html"] = Buffer.from(entryPage(info, true));
  const kept = reachableFiles(files, { ...info, origin });
  const artifact = Object.fromEntries([...kept].map((path) => [path, files[path]]));
  // Validate before replacing a previous output; a bad input leaves it untouched.
  verifyPages(artifact, info, { origin, repository });
  await rm(output, { recursive: true, force: true });
  for (const [path, bytes] of Object.entries(artifact)) {
    await mkdir(dirname(join(output, path)), { recursive: true });
    await writeFile(join(output, path), bytes);
  }
  const listing = Object.keys(artifact).sort();
  console.log(`Blog-only Pages artifact (${listing.length} files):\n${listing.join("\n")}`);
  return listing;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error("Usage: pages.mjs");
  await prunePages();
}
