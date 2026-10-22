// Temporary #4443 hosted diagnostic. No API calls, release dispatch or publication.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { release } from "node:os";
import { fileURLToPath } from "node:url";

const sha = "a".repeat(40);
const tag = "v5.0.0-alpha.999999";
// The captured 961-release catalogue is 14,546,406 bytes. This is over twice that size.
const input = Buffer.from(
  JSON.stringify([
    [
      {
        tag_name: tag,
        target_commitish: sha,
        draft: true,
        created_at: "2026-10-22T00:00:00Z",
        assets: [],
        body: `Unicode: 日本語 🧪 ${"x".repeat(32 * 1024 * 1024)}`,
      },
    ],
  ]),
);
const digest = createHash("sha256").update(input).digest("hex");
const finish = `const bytes = Buffer.isBuffer(value) ? value : Buffer.from(value); JSON.parse(bytes.toString('utf8')); console.log(JSON.stringify({bytes:bytes.length,sha256:require('node:crypto').createHash('sha256').update(bytes).digest('hex')}));`;
const cases = [
  ["readFileSync-utf8", ["-e", `const value=require('node:fs').readFileSync(0,'utf8');${finish}`]],
  ["readFileSync-buffer", ["-e", `const value=require('node:fs').readFileSync(0);${finish}`]],
  [
    "readSync-chunks",
    [
      "-e",
      `const {readSync}=require('node:fs'); const chunks=[]; for (;;) { const b=Buffer.allocUnsafe(65536); const n=readSync(0,b,0,b.length,null); if(n===0)break; chunks.push(b.subarray(0,n)); } const value=Buffer.concat(chunks);${finish}`,
    ],
  ],
  [
    "planner",
    [
      fileURLToPath(new URL("./plan-release-builds.mjs", import.meta.url)),
      "--product",
      "cli",
      "--tag",
      tag,
    ],
  ],
];
const results = cases.map(([name, args]) => {
  const started = performance.now();
  // spawnSync's input uses the child's stdin pipe, never a regular-file descriptor.
  const child = spawnSync(process.execPath, args, {
    input,
    encoding: "utf8",
    timeout: 60_000,
    maxBuffer: 1024 * 1024,
    env: { ...process.env, GITHUB_OUTPUT: "", GITHUB_STEP_SUMMARY: "" },
  });
  let valid = false;
  let validationError = "";
  if (child.status === 0) {
    try {
      if (name === "planner") {
        const lines = child.stdout.trim().split("\n");
        assert.deepEqual(JSON.parse(lines.find((line) => line.startsWith("matrix=")).slice(7)), {
          include: [{ tag, sha }],
        });
        assert(lines.includes("any=true"));
      } else {
        assert.deepEqual(JSON.parse(child.stdout), { bytes: input.length, sha256: digest });
      }
      valid = true;
    } catch (error) {
      validationError = error.message;
    }
  }
  return {
    name,
    status: child.status,
    signal: child.signal,
    error: child.error?.message ?? "",
    valid,
    validationError,
    seconds: (performance.now() - started) / 1000,
    stderr: child.stderr?.slice(-2000) ?? "",
  };
});
const report = {
  node: process.version,
  platform: process.platform,
  arch: process.arch,
  kernel: release(),
  image: process.env.ImageOS,
  imageVersion: process.env.ImageVersion,
  bytes: input.length,
  sha256: digest,
  transport: "spawnSync stdin pipe",
  results,
};
mkdirSync("stdin-probe-results", { recursive: true });
writeFileSync("stdin-probe-results/report.json", `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify(report, null, 2));
// A reproduced crash is diagnostic evidence, not a reason to suppress the other matrix versions.
if (results.some((row) => row.status === 0 && !row.valid)) process.exitCode = 1;
