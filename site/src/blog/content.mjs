import { createHash } from "node:crypto";
import { parse, parseFragment, serialize, serializeOuter } from "parse5";

export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const stableJson = (value) => JSON.stringify(value, null, 2) + "\n";
export const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const hash = /^[a-f0-9]{64}$/;
const slugPattern = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const tags = new Set(
  "article section header footer h1 h2 h3 h4 p a ul ol li blockquote pre code strong em b i s del small sub sup span div br hr figure figcaption img table thead tbody tr th td caption details summary button input label output select option fieldset legend canvas svg g path circle rect line polyline polygon text title style script".split(
    " ",
  ),
);
const forbiddenAttributes = /^(?:on|data-(?:tmt|colab)-)/i;
const attrNames = new Set(
  "id class title role aria-label aria-labelledby aria-describedby aria-live aria-hidden aria-controls aria-expanded tabindex type value min max step checked disabled selected name for width height alt src href loading rel scope colspan rowspan viewBox d fill stroke stroke-width cx cy r x y x1 x2 y1 y2 points xmlns style data-demo".split(
    " ",
  ),
);

export function walk(node, visit) {
  visit(node);
  for (const child of node.childNodes ?? []) walk(child, visit);
}

export function attr(node, name) {
  return node.attrs?.find((item) => item.name === name)?.value;
}

export function rasterExtension(bytes) {
  if (bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return "png";
  if (bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255) return "jpg";
  if (bytes.subarray(0, 4).toString() === "RIFF" && bytes.subarray(8, 12).toString() === "WEBP")
    return "webp";
  throw new Error("Attachment is not a supported raster");
}

export function metadata(value) {
  const keys = ["slug", "title", "summary", "date", "language"];
  if (!value || Object.keys(value).sort().join() !== keys.sort().join())
    throw new Error("Metadata requires exactly slug, title, summary, date, language");
  if (!slugPattern.test(value.slug) || value.slug.length > 100)
    throw new Error("Invalid post slug");
  for (const [key, limit] of [
    ["title", 160],
    ["summary", 400],
  ]) {
    if (
      typeof value[key] !== "string" ||
      !value[key].trim() ||
      value[key].length > limit ||
      [...value[key]].some((character) => character.charCodeAt(0) < 32)
    )
      throw new Error(`Invalid ${key}`);
  }
  if (
    !/^\d{4}-\d{2}-\d{2}$/.test(value.date) ||
    new Date(value.date).toISOString().slice(0, 10) !== value.date
  )
    throw new Error("Invalid post date");
  if (value.language !== "en")
    throw new Error(
      "Posts are English-only; add a repository language exception before enabling another language",
    );
  return value;
}

export function articleFromSource(source) {
  if (Buffer.byteLength(source) > 2 * 1024 * 1024) throw new Error("Page exceeds 2 MiB");
  const document = parse(source);
  const articles = [];
  walk(document, (node) => {
    if (node.tagName === "article") articles.push(node);
  });
  if (articles.length !== 1 || attr(articles[0], "id") !== "blog-article")
    throw new Error("Source needs exactly one article#blog-article");
  return serializeOuter(articles[0]);
}

// Intentional code is extracted from the approved article into local modules.
// Everything remaining is a closed HTML vocabulary, not arbitrary app chrome.
export function extractArticle(raw, images = new Map()) {
  const document = parseFragment(raw);
  walk(document, (node) => {
    if (node.childNodes)
      node.childNodes = node.childNodes.filter((child) => child.nodeName !== "#comment");
  });
  const roots = document.childNodes.filter(
    (node) => node.nodeName !== "#text" || node.value.trim(),
  );
  if (
    roots.length !== 1 ||
    roots[0].tagName !== "article" ||
    attr(roots[0], "id") !== "blog-article"
  )
    throw new Error("Only one article#blog-article may be imported");
  const scripts = [];
  const assets = {};
  walk(document, (node) => {
    if (!node.tagName) return;
    if (!tags.has(node.tagName)) throw new Error(`Unsupported article element: ${node.tagName}`);
    if (
      node.namespaceURI !== "http://www.w3.org/1999/xhtml" &&
      node.namespaceURI !== "http://www.w3.org/2000/svg"
    )
      throw new Error("Unsupported namespace");
    if (node.tagName === "script") {
      if (
        attr(node, "type") !== "module" ||
        attr(node, "src") !== undefined ||
        node.attrs.length !== 1
      )
        throw new Error("Author scripts must be inline type=module; no external script origins");
      const code = node.childNodes.map((child) => child.value ?? "").join("");
      const digest = sha256(code);
      const path = `scripts/${digest}.mjs`;
      assets[path] = Buffer.from(code);
      scripts.push({ path, sha256: digest });
      node.attrs.push({ name: "src", value: `./${path}` });
      node.childNodes = [];
      return;
    }
    for (const item of node.attrs ?? []) {
      if (
        forbiddenAttributes.test(item.name) ||
        !attrNames.has(item.name) ||
        /(?:^|\s)(?:colab|tmt)-(?:annotation|discussion|proposal|comment)/i.test(item.value)
      )
        throw new Error(`Unsupported article attribute: ${item.name}`);
      if (item.name === "href" && !/^(?:https:\/\/[^\s]+|#[a-zA-Z][\w-]*)$/.test(item.value))
        throw new Error("Only HTTPS and fragment links are allowed");
      if (item.name === "src") {
        if (node.tagName !== "img" || !images.has(item.value))
          throw new Error("Images must be verified Colab raster attachments");
        const image = images.get(item.value);
        if (
          !["png", "jpg", "webp"].includes(image.extension) ||
          rasterExtension(image.bytes) !== image.extension
        )
          throw new Error("Only raster images are allowed");
        const path = `images/${sha256(image.bytes)}.${image.extension}`;
        assets[path] = image.bytes;
        item.value = `./${path}`;
      }
    }
    if (node.tagName === "img" && attr(node, "alt") === undefined)
      throw new Error("Images need alt text");
    if (
      node.tagName === "input" &&
      !["range", "number", "checkbox", "radio", "button"].includes(attr(node, "type"))
    )
      throw new Error("Unsupported input type");
    if (node.tagName === "style") {
      const css = node.childNodes.map((child) => child.value ?? "").join("");
      if (/@import|url\s*\(/i.test(css)) throw new Error("Article CSS must not load resources");
    }
  });
  return { html: serialize(document).trimEnd() + "\n", assets, scripts };
}

export function artifactHash(meta, html, assets) {
  return sha256(
    stableJson({
      metadata: metadata(meta),
      html,
      assets: Object.fromEntries(
        Object.entries(assets)
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([path, bytes]) => [path, sha256(bytes)]),
      ),
    }),
  );
}

export function validateStoredArticle(html, assets) {
  const document = parseFragment(html);
  const used = new Set();
  // Round-trip through the same extractor using reconstructed inline modules and
  // verified image slots; this also rejects hand-added event handlers/frames.
  walk(document, (node) => {
    if (node.tagName === "script") {
      const path = attr(node, "src")?.replace(/^\.\//, "");
      if (
        attr(node, "type") !== "module" ||
        node.attrs.length !== 2 ||
        !/^scripts\/[a-f0-9]{64}\.mjs$/.test(path ?? "") ||
        !assets[path] ||
        sha256(assets[path]) !== path.slice(8, -4)
      )
        throw new Error("Invalid committed script");
      used.add(path);
      node.attrs = [{ name: "type", value: "module" }];
      node.childNodes = [
        { nodeName: "#text", value: assets[path].toString("utf8"), parentNode: node },
      ];
    }
    if (node.tagName === "img") {
      const path = attr(node, "src")?.replace(/^\.\//, "");
      if (
        !/^images\/[a-f0-9]{64}\.(?:png|jpg|webp)$/.test(path ?? "") ||
        !assets[path] ||
        sha256(assets[path]) !== path.slice(7, 71)
      )
        throw new Error("Invalid committed image");
      used.add(path);
    }
  });
  const images = new Map(
    [...used]
      .filter((path) => path.startsWith("images/"))
      .map((path) => [`./${path}`, { bytes: assets[path], extension: path.split(".").at(-1) }]),
  );
  const restored = serialize(document);
  const result = extractArticle(restored, images);
  if (result.html !== html || Object.keys(assets).some((path) => !used.has(path)))
    throw new Error("Article is not canonical or has unreferenced assets");
  return result;
}

export function approvedRecord(record, snapshot, proposals) {
  // Trusted decisions advance the document revision without changing its source.
  if (snapshot.pageId !== record.pageId || sha256(snapshot.source) !== record.sourceSha256)
    throw new Error("Page source drift: prepare a new proposal");
  if (proposals.pageId !== record.pageId || proposals.epoch !== snapshot.epoch)
    throw new Error("Proposal page/epoch mismatch");
  const rows = proposals.proposals.filter((row) => row.proposal?.proposalId === record.proposalId);
  if (
    rows.length !== 1 ||
    rows[0].deleted ||
    rows[0].proposal.body !== record.proposalBody ||
    rows[0].proposal.title !== "Publish this post?"
  )
    throw new Error("Missing or changed publish proposal");
  const decision = rows[0].decision;
  if (
    !decision ||
    decision.decision !== "approved" ||
    decision.deleted ||
    decision.kind !== "proposal-decision" ||
    decision.pageId !== record.pageId ||
    decision.epoch !== snapshot.epoch ||
    !uuid.test(decision.senderDevice) ||
    !uuid.test(decision.actionId)
  )
    throw new Error(
      "A trusted owner device must approve the proposal; Ben must perform the approval",
    );
  return decision;
}

export function validateApproval(approval, meta, html, assets) {
  const approvedAt = approval?.approvedAt;
  const timestamp = /^\d+$/.test(approvedAt ?? "") ? Number(approvedAt) : Date.parse(approvedAt);
  if (
    !approval ||
    approval.decision !== "approved" ||
    !uuid.test(approval.pageId) ||
    !uuid.test(approval.proposalId) ||
    !uuid.test(approval.approverDevice) ||
    !uuid.test(approval.actionId) ||
    !hash.test(approval.sourceSha256 ?? "") ||
    typeof approval.pageRevision !== "string" ||
    !approval.pageRevision ||
    typeof approvedAt !== "string" ||
    !Number.isSafeInteger(timestamp) ||
    !Number.isFinite(new Date(timestamp).getTime())
  )
    throw new Error("Missing or invalid approval evidence");
  if (approval.artifactSha256 !== artifactHash(meta, html, assets))
    throw new Error("Approved artifact hash mismatch");
}
