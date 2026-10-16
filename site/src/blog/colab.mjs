import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { randomUUID } from "node:crypto";
import { readFile, writeFile, mkdir, mkdtemp, rename, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "parse5";
import {
  approvedRecord,
  articleFromSource,
  artifactHash,
  attr,
  extractArticle,
  metadata,
  rasterExtension,
  sha256,
  stableJson,
  uuid,
  walk,
} from "./content.mjs";
import { escapeHtml } from "./site.mjs";

const execute = promisify(execFile);
export async function tmt(...args) {
  const executable = process.env.TMT_EXECUTABLE || "tmt";
  try {
    const { stdout } = await execute(executable, ["colab", ...args, "--json"], {
      timeout: 30_000,
      maxBuffer: 10 * 1024 * 1024,
    });
    return JSON.parse(stdout);
  } catch (error) {
    let detail = error.stderr || error.message;
    try {
      detail = JSON.parse(error.stdout).error?.message ?? detail;
    } catch {}
    throw new Error(
      `Colab command failed (${executable}): ${detail}. Verify the native CLI with tmt colab --help, or set TMT_EXECUTABLE to its absolute path. Do not fall back to the retired runtime.`,
      { cause: error },
    );
  }
}

export async function draft({ root, sourcePath, meta, call = tmt }) {
  metadata(meta);
  const article = articleFromSource(await readFile(sourcePath, "utf8"));
  const css = await readFile(join(root, "src/blog/blog.css"), "utf8");
  const e = escapeHtml;
  const source = `<!doctype html><html lang="${meta.language}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${e(meta.title)}</title><style>${css}</style></head><body><main class="post-shell"><header class="post-heading"><p class="eyebrow">ARTICLE DRAFT / ${meta.date}</p><h1>${e(meta.title)}</h1><p>${e(meta.summary)}</p></header>${article}</main></body></html>`;
  const directory = join(root, ".private/drafts");
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const path = join(directory, randomUUID() + ".html");
  await writeFile(path, source, { mode: 0o600, flag: "wx" });
  const created = await call(
    "page",
    "create",
    "--title",
    meta.title,
    "--file",
    path,
    "--no-open",
    "--strict",
  );
  if (!uuid.test(created.pageId))
    throw new Error("Uncertain Colab creation; inspect page list before retrying");
  await call("label", "add", created.pageId, "blog");
  return created;
}

export async function capture(pageId, call = tmt) {
  const snapshot = await call("page", "read", pageId);
  if (
    snapshot.pageId !== pageId ||
    typeof snapshot.revision !== "string" ||
    typeof snapshot.source !== "string" ||
    typeof snapshot.epoch !== "string"
  )
    throw new Error("Invalid Colab source response");
  const raw = articleFromSource(snapshot.source);
  const references = new Set();
  walk(parse(raw), (node) => {
    if (node.tagName === "img") references.add(attr(node, "src"));
  });
  if (references.size > 16) throw new Error("An article may reference at most 16 images");
  const images = new Map();
  const temporary = await mkdtemp(join(tmpdir(), "tmt-blog-attachments-"));
  let totalBytes = 0;
  try {
    for (const reference of references) {
      const attachmentId = reference?.replace(/^colab-attachment:/, "");
      if (!reference?.startsWith("colab-attachment:") || !uuid.test(attachmentId))
        throw new Error("Use full verified Colab attachment IDs for images");
      const image = await call("attachment", "read", pageId, attachmentId, "--output", temporary);
      if (
        typeof image.path !== "string" ||
        !resolve(image.path).startsWith(temporary + "/") ||
        !(await stat(image.path)).isFile()
      )
        throw new Error("Invalid attachment path");
      const bytes = await readFile(image.path);
      totalBytes += bytes.length;
      if (
        bytes.length > 16 * 1024 * 1024 ||
        totalBytes > 64 * 1024 * 1024 ||
        sha256(bytes) !== image.sha256
      )
        throw new Error("Attachment integrity/size mismatch");
      images.set(reference, { bytes, extension: rasterExtension(bytes) });
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
  return { snapshot, ...extractArticle(raw, images) };
}

export async function prepare({ root, pageId, meta, call = tmt }) {
  if (!uuid.test(pageId)) throw new Error("Use a full page UUID");
  metadata(meta);
  const first = await capture(pageId, call);
  const artifactSha256 = artifactHash(meta, first.html, first.assets);
  const proposalId = randomUUID();
  const inventory = first.scripts.length
    ? first.scripts.map(({ path, sha256 }) => `- ${path}: SHA-256 ${sha256}`).join("\n")
    : "- None";
  const proposalBody = `Publish this post?\n\nTitle: ${meta.title}\nSlug: ${meta.slug}\nDate: ${meta.date}\nLanguage: ${meta.language}\nSummary: ${meta.summary}\n\nApproved artifact SHA-256: ${artifactSha256}\n\nArticle modules (exact source is in this page):\n${inventory}\n\nExternal resource/data origins: none allowed by the site CSP. Outbound HTTPS links remain reader actions.\n\nApprove authorizes importing this exact article, scripts, images and metadata into the blog repository for PR review and merge-triggered publication. Discussions and proposals are excluded. Any source revision change requires a new proposal.`;
  if (Buffer.byteLength(proposalBody) > 4096)
    throw new Error("Proposal inventory exceeds Colab limit; reduce scripts or split the post");
  const directory = join(root, ".private/proposals");
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const path = join(directory, proposalId + ".json");
  const record = { pageId, proposalId, meta, artifactSha256, proposalBody, state: "preparing" };
  await writeFile(path, stableJson(record), { mode: 0o600, flag: "wx" });
  const added = await call(
    "proposal",
    "add",
    pageId,
    "--id",
    proposalId,
    "--title",
    "Publish this post?",
    "--body",
    proposalBody,
  );
  if (added.proposalId !== proposalId || added.placed !== true)
    throw new Error(
      `Proposal placement uncertain. Inspect ${pageId}/${proposalId}; do not create a duplicate`,
    );
  const current = await capture(pageId, call);
  if (artifactHash(meta, current.html, current.assets) !== artifactSha256)
    throw new Error("Article changed while preparing; this proposal must not be published");
  Object.assign(record, {
    state: "awaiting-approval",
    pageRevision: current.snapshot.revision,
    sourceSha256: sha256(current.snapshot.source),
  });
  await writeFile(path, stableJson(record), { mode: 0o600 });
  return { pageId, proposalId, pageRevision: record.pageRevision, artifactSha256, record: path };
}

export async function publish({ root, proposalId, call = tmt }) {
  if (!uuid.test(proposalId)) throw new Error("Use the full proposal UUID");
  const record = JSON.parse(
    await readFile(join(root, ".private/proposals", proposalId + ".json"), "utf8"),
  );
  if (record.state !== "awaiting-approval" || record.proposalId !== proposalId)
    throw new Error("Proposal preparation is incomplete");
  const captured = await capture(record.pageId, call);
  const proposals = await call("proposal", "ls", record.pageId);
  const decision = approvedRecord(record, captured.snapshot, proposals);
  if (artifactHash(record.meta, captured.html, captured.assets) !== record.artifactSha256)
    throw new Error("Approved article or image bytes changed");
  const confirmed = await call("page", "read", record.pageId);
  approvedRecord(record, confirmed, proposals);
  const approval = {
    decision: "approved",
    pageId: record.pageId,
    proposalId,
    pageRevision: record.pageRevision,
    sourceSha256: record.sourceSha256,
    approverDevice: decision.senderDevice,
    actionId: decision.actionId,
    approvedAt: decision.at,
    artifactSha256: record.artifactSha256,
  };
  const staging = await mkdtemp(join(root, ".private/post-"));
  let committed = false;
  try {
    const files = {
      "meta.json": stableJson(record.meta),
      "article.html": captured.html,
      "approval.json": stableJson(approval),
      ...captured.assets,
    };
    for (const [path, bytes] of Object.entries(files)) {
      await mkdir(dirname(join(staging, path)), { recursive: true });
      await writeFile(join(staging, path), bytes);
    }
    const destination = join(root, "src/posts", record.meta.slug);
    // Refuse replacement so an existing published post is never silently lost.
    // Revisions are prepared as a reviewed Git change using the documented flow.
    try {
      await stat(destination);
      throw new Error("Post already exists; use the documented revision workflow");
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    await mkdir(dirname(destination), { recursive: true });
    await rename(staging, destination);
    committed = true;
    return { slug: record.meta.slug, path: destination, artifactSha256: record.artifactSha256 };
  } finally {
    if (!committed) await rm(staging, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [command, first, second] = process.argv.slice(2);
  let result;
  if (command === "draft" && first && second)
    result = await draft({
      root: resolve("."),
      sourcePath: first,
      meta: JSON.parse(await readFile(second, "utf8")),
    });
  else if (command === "prepare" && first && second)
    result = await prepare({
      root: resolve("."),
      pageId: first,
      meta: JSON.parse(await readFile(second, "utf8")),
    });
  else if (command === "publish" && first && !second)
    result = await publish({ root: resolve("."), proposalId: first });
  else
    throw new Error(
      "Usage: pnpm blog:colab draft source.html metadata.json | pnpm blog:colab prepare PAGE_UUID metadata.json | pnpm blog:colab publish PROPOSAL_UUID",
    );
  console.log(stableJson(result));
}
