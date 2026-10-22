import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { capture, prepare, publish } from "../src/blog/colab.mjs";
import { readPosts } from "../src/blog/site.mjs";

const pageId = "10000000-0000-4000-8000-000000000001";
const device = "30000000-0000-4000-8000-000000000001";
const actionId = "40000000-0000-4000-8000-000000000001";
const meta = {
  slug: "approved-post",
  title: "Approved title",
  summary: "Summary",
  date: "2026-10-16",
  language: "en",
};

function fakeColab() {
  const state = {
    source:
      '<article id="blog-article"><p>Public body</p><script type="module">document.body.dataset.example = "yes";</script></article><aside>Private note</aside>',
    revision: "before",
    decision: null,
    calls: [],
  };
  const call = async (...args) => {
    state.calls.push(args);
    if (args[0] === "page" && args[1] === "read")
      return { pageId, epoch: "1", revision: state.revision, source: state.source };
    if (args[0] === "proposal" && args[1] === "add") {
      state.id = args[args.indexOf("--id") + 1];
      state.body = args[args.indexOf("--body") + 1];
      state.source += `<tmt-proposal data-id="${state.id}"></tmt-proposal>`;
      state.revision = "after-proposal-placement";
      return { proposalId: state.id, placed: true };
    }
    if (args[0] === "proposal" && args[1] === "ls")
      return {
        pageId,
        epoch: "1",
        proposals: [
          {
            deleted: false,
            proposal: { proposalId: state.id, title: "Publish this post?", body: state.body },
            decision: state.decision,
          },
        ],
      };
    throw new Error("Unexpected native call: " + args.join(" "));
  };
  const approve = () => {
    state.revision = "after-trusted-decision";
    state.decision = {
      decision: "approved",
      deleted: false,
      kind: "proposal-decision",
      pageId,
      epoch: "1",
      senderDevice: device,
      actionId,
      at: "1792152000000",
    };
  };
  return { state, call, approve };
}

async function world(run) {
  const root = await mkdtemp(join(tmpdir(), "tmt-blog-test-"));
  await mkdir(join(root, "src/posts"), { recursive: true });
  try {
    await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("prepared proposal inventories exact scripts; trusted approval imports only public output", async () =>
  world(async (root) => {
    const fake = fakeColab();
    const result = await prepare({ root, pageId, meta, call: fake.call });
    assert.match(fake.state.body, /Article modules/);
    assert.match(fake.state.body, /scripts\/[a-f0-9]{64}\.mjs/);
    assert.match(fake.state.body, /External resource\/data origins: none/);
    assert.equal(result.pageRevision, "after-proposal-placement");
    const saved = await readFile(result.record, "utf8");
    assert.doesNotMatch(saved, /Public body|Private note|document.body/);
    fake.approve();
    await publish({ root, proposalId: result.proposalId, call: fake.call });
    const posts = await readPosts(join(root, "src/posts"));
    assert.equal(posts.length, 1);
    assert.doesNotMatch(posts[0].html, /Private note|tmt-proposal/);
    assert.equal(Object.keys(posts[0].assets).length, 1);
    assert.equal(
      fake.state.calls.some((args) => ["export", "threads"].includes(args[0])),
      false,
    );
    assert.equal(
      (await readdir(join(root, ".private"))).some((path) => path.startsWith("post-")),
      false,
    );
  }));

test("pending and declined decisions refuse without public files", async () =>
  world(async (root) => {
    const fake = fakeColab();
    const result = await prepare({ root, pageId, meta, call: fake.call });
    await assert.rejects(
      publish({ root, proposalId: result.proposalId, call: fake.call }),
      /must approve/,
    );
    fake.approve();
    fake.state.decision.decision = "declined";
    await assert.rejects(
      publish({ root, proposalId: result.proposalId, call: fake.call }),
      /must approve/,
    );
    assert.deepEqual(await readdir(join(root, "src/posts")), []);
  }));

test("trusted approval may advance the document revision while source stays identical", async () =>
  world(async (root) => {
    const fake = fakeColab();
    const result = await prepare({ root, pageId, meta, call: fake.call });
    const preparedSource = fake.state.source;
    fake.approve();
    assert.notEqual(fake.state.revision, result.pageRevision);
    assert.equal(fake.state.source, preparedSource);
    await publish({ root, proposalId: result.proposalId, call: fake.call });
    const posts = await readPosts(join(root, "src/posts"));
    assert.equal(posts.length, 1);
    assert.match(posts[0].html, /Public body/);
    const approval = JSON.parse(
      await readFile(join(root, "src/posts", meta.slug, "approval.json"), "utf8"),
    );
    const record = JSON.parse(await readFile(result.record, "utf8"));
    assert.equal(approval.pageRevision, result.pageRevision);
    assert.equal(approval.sourceSha256, record.sourceSha256);
    assert.equal(approval.artifactSha256, result.artifactSha256);
    assert.equal(approval.actionId, actionId);
    assert.equal(approval.approvedAt, fake.state.decision.at);
  }));

test("changed full source refuses even with the prepared revision and unchanged article", async () =>
  world(async (root) => {
    const fake = fakeColab();
    const result = await prepare({ root, pageId, meta, call: fake.call });
    fake.approve();
    fake.state.revision = result.pageRevision;
    fake.state.source = fake.state.source.replace("Private note", "Changed private note");
    await assert.rejects(
      publish({ root, proposalId: result.proposalId, call: fake.call }),
      /source drift/,
    );
    assert.deepEqual(await readdir(join(root, "src/posts")), []);
  }));

test("changed bytes, missing proposal and wrong epoch each refuse independently", async () => {
  for (const mutate of [
    (fake) => {
      fake.state.source = fake.state.source.replace("Public body", "Changed body");
    },
    (fake) => {
      fake.state.id = "20000000-0000-4000-8000-000000000002";
    },
    (fake) => {
      fake.state.decision.epoch = "2";
    },
  ])
    await world(async (root) => {
      const fake = fakeColab();
      const result = await prepare({ root, pageId, meta, call: fake.call });
      fake.approve();
      mutate(fake);
      await assert.rejects(publish({ root, proposalId: result.proposalId, call: fake.call }));
      assert.deepEqual(await readdir(join(root, "src/posts")), []);
    });
});

test("capture rejects remote images before any attachment call", async () => {
  const fake = fakeColab();
  fake.state.source =
    '<article id="blog-article"><img src="https://example.com/x.png" alt="x"></article>';
  await assert.rejects(capture(pageId, fake.call), /full verified Colab attachment IDs/);
  assert.equal(
    fake.state.calls.some((args) => args[0] === "attachment"),
    false,
  );
});

test("repeated import refuses to overwrite existing post and cleans staging", async () =>
  world(async (root) => {
    const fake = fakeColab();
    const result = await prepare({ root, pageId, meta, call: fake.call });
    fake.approve();
    await publish({ root, proposalId: result.proposalId, call: fake.call });
    await assert.rejects(
      publish({ root, proposalId: result.proposalId, call: fake.call }),
      /already exists/,
    );
    assert.equal((await readPosts(join(root, "src/posts"))).length, 1);
    assert.equal(
      (await readdir(join(root, ".private"))).some((path) => path.startsWith("post-")),
      false,
    );
  }));
