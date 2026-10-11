/** Release-supplied data, bound to its exact bytes; no extension-specific resource names. */
import { RefusalError } from './operations.js';
export interface HostedDeclaration {
  readonly bytes: string;
  readonly digest: string;
}
// Native declaration.rs owns the grammar; tests/deploy_plan.rs pins its reserved set.
const name = /^[a-z][a-z0-9-]{0,31}$/;
const field = /^[A-Za-z][A-Za-z0-9]{0,63}$/;
function require(condition: boolean): asserts condition {
  if (!condition) throw new RefusalError('REMOTE_INPUT_INVALID');
}
function members(value: unknown, keys: string[]): Record<string, unknown> {
  require(typeof value === 'object' && value !== null && !Array.isArray(value));
  const row = value as Record<string, unknown>;
  require(Object.keys(row).sort().join(',') === keys.sort().join(','));
  return row;
}
export async function verifyHostedDeclaration(input: HostedDeclaration): Promise<void> {
  try {
    require(typeof input.bytes === 'string' && typeof input.digest === 'string');
    const bytes = new TextEncoder().encode(input.bytes);
    require(bytes.length <= 65536 && /^[a-f0-9]{64}$/.test(input.digest));
    const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), (b) =>
      b.toString(16).padStart(2, '0'),
    ).join('');
    require(digest === input.digest);
    const value: unknown = JSON.parse(input.bytes);
    require(typeof value === 'object' && value !== null && !Array.isArray(value));
    const row = value as Record<string, unknown>;
    members(row, [
      'version',
      'extension',
      'backend',
      'resources',
      'admission',
      ...(Object.hasOwn(row, 'hosting') ? ['hosting'] : []),
    ]);
    require(
      row.version === 1 &&
        row.backend === 'firestore' &&
        typeof row.extension === 'string' &&
        name.test(row.extension),
    );
    require(Array.isArray(row.resources) && row.resources.length <= 64);
    const names = new Set<string>();
    for (const value of row.resources) {
      const r = members(value, ['name', 'kind', 'path', 'limits', 'ttlField', 'indexes']);
      require(typeof r.name === 'string' && name.test(r.name) && !names.has(r.name));
      names.add(r.name);
      require(
        typeof r.kind === 'string' && ['log', 'checkpoint', 'blob', 'awareness'].includes(r.kind),
      );
      require(typeof r.path === 'string');
      const segments = r.path.split('/');
      require(segments.length <= 8 && segments.every((s) => /^[a-z0-9_-]{1,64}$/.test(s)));
      require(!['x', 'append', 'subscribe', 'ack', 'pair'].includes(segments[0]!));
      const limits = members(r.limits, ['maxObjectBytes', 'maxNamespaceBytes', 'maxEntries']);
      require(
        Object.values(limits).every(
          (n) => typeof n === 'number' && Number.isSafeInteger(n) && n > 0,
        ),
      );
      require(Number(limits.maxObjectBytes) <= Number(limits.maxNamespaceBytes));
      require(r.ttlField === null || (typeof r.ttlField === 'string' && field.test(r.ttlField)));
      require(Array.isArray(r.indexes) && r.indexes.length <= 64);
      const indexes = new Set<string>();
      for (const value of r.indexes) {
        const index = members(value, ['field', 'direction']);
        require(
          typeof index.field === 'string' &&
            field.test(index.field) &&
            typeof index.direction === 'string' &&
            ['asc', 'desc'].includes(index.direction),
        );
        const key = `${index.field}:${index.direction}`;
        require(!indexes.has(key));
        indexes.add(key);
      }
      require(r.kind !== 'awareness' || (r.ttlField === null && r.indexes.length === 0));
    }
    const admission = members(row.admission, ['artifact', 'digest', 'entryPoint']);
    require(
      typeof admission.artifact === 'string' &&
        admission.artifact.split('/').length <= 8 &&
        admission.artifact.split('/').every((s) => /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(s)),
    );
    require(typeof admission.digest === 'string' && /^[a-f0-9]{64}$/.test(admission.digest));
    require(
      typeof admission.entryPoint === 'string' &&
        /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(admission.entryPoint),
    );
    // Hosting describes deploy inputs, not Auth or resource authority. No consumer here.
  } catch {
    throw new RefusalError('REMOTE_INPUT_INVALID');
  }
}
