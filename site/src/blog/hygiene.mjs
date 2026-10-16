import { execFileSync } from "node:child_process";

export function privatePath(path) {
  return /(?:^|\/)(?:\.private|exports?|colab-export[^/]*|\.venv|node_modules|dist|\.generated|attachments)(?:\/|$)|(?:^|\/)(?:conversations\.(?:json|md)|page\.html|manifest\.json)$/.test(
    path,
  );
}

const tracked = execFileSync("git", ["ls-files", "-z", "--", "."], { encoding: "utf8" })
  .split("\0")
  .filter(Boolean);
for (const path of tracked)
  if (privatePath(path)) throw new Error(`Private or generated path is tracked: ${path}`);
console.log("No Colab exports, discussions or local runtime files are tracked.");
