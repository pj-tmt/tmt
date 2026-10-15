import { createHash } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import path from 'node:path';
import { expect } from '@playwright/test';
import { run } from './ask.js';
import type { AcceptanceWorld } from './world.js';

interface DeclaredFile {
  path: string;
  sha256: string;
  length: number;
  contentType: string;
  csp?: string;
}
interface Composition {
  version: number;
  files: Array<{ path: string; rawDigest: string; rawLength: number; contentType: string }>;
  config: {
    headers: Array<{ glob?: string; regex?: string; headers: Record<string, string> }>;
    rewrites: Array<{ regex: string; path: string }>;
  };
  digest: string;
}

export const sha256 = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');

/** The two public replies Remote reads before any provider call, then Remote's own composition. */
export function composed(world: AcceptanceWorld) {
  const declaration = run(world, world.binaries.colab, ['deploy-declaration', '--json']);
  const bundle = run(world, world.binaries.colab, ['hosting-bundle', '--json']);
  const example = path.join(path.dirname(world.binaries.colab), 'examples', 'compose_hosting');
  if (!fs.existsSync(example))
    throw new Error(
      `Missing ${example}. Build it in its own invocation, after the three executables: cd rust && cargo build --locked -p tmt-remote --example compose_hosting. Never add it to the executables' build: an example selected next to tmt-cli turns on its dev-dependency features and the resulting tmt refuses to run in a world (or set TMT_ACCEPTANCE_BIN_DIR to a directory that holds examples/compose_hosting).`,
    );
  const declarationFile = path.join(world.root, 'declaration-envelope.json');
  const bundleFile = path.join(world.root, 'hosting-bundle-reply.json');
  fs.writeFileSync(declarationFile, declaration);
  fs.writeFileSync(bundleFile, bundle);
  const view = JSON.parse(run(world, example, [declarationFile, bundleFile])) as Composition;
  const manifest = (JSON.parse(JSON.parse(declaration).declaration) as { hosting: unknown })
    .hosting as { version: number; files: DeclaredFile[] };
  const files = new Map<string, Buffer>();
  for (const file of JSON.parse(bundle).files as Array<{ path: string; bytesBase64: string }>)
    files.set(file.path, Buffer.from(file.bytesBase64, 'base64'));
  // Remote composes this exact raw declaration alongside the extension's declared files.
  const rawDeclaration = Buffer.from(JSON.parse(declaration).declaration, 'utf8');
  const declarationPath = '/tmt/declarations/colab.json';
  const declared = view.files.find((file) => file.path === declarationPath)!;
  expect(declared.rawDigest).toBe(sha256(rawDeclaration));
  expect(declared.rawLength).toBe(rawDeclaration.byteLength);
  files.set(declarationPath, rawDeclaration);
  return { view, manifest, files };
}

interface Served {
  origin: string;
  /** Every path the server answered, in order; the server is the only network a page has. */
  requests: string[];
}

/** Serve the declared bytes with the composed headers; the shell for the composed rewrites. */
export async function serve(
  world: AcceptanceWorld,
  view: Composition,
  files: Map<string, Buffer>,
): Promise<Served> {
  const requests: string[] = [];
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url ?? '/', 'http://127.0.0.1').pathname;
    requests.push(pathname);
    const rewrite = view.config.rewrites.find((entry) => new RegExp(entry.regex).test(pathname));
    const target = pathname === '/' ? '/index.html' : (rewrite?.path ?? pathname);
    const bytes = files.get(target);
    if (bytes === undefined) {
      response.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      response.end('not found');
      return;
    }
    const headers: Record<string, string> = {};
    for (const entry of view.config.headers)
      if (entry.glob === pathname || (entry.regex && new RegExp(entry.regex).test(pathname)))
        Object.assign(headers, entry.headers);
    response.writeHead(200, headers);
    response.end(bytes);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  world.onDispose(async (stage) => {
    stage('close the hosted bundle server');
    await new Promise<void>((resolve) => {
      server.close(() => resolve());
      server.closeAllConnections();
    });
  });
  return { origin: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, requests };
}
