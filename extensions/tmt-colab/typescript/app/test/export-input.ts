import { readFileSync } from 'node:fs';
import type { ExportView } from '../src/export.js';
import type { ExportAttachment } from '../src/export-attachments.js';

export const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/export-v1.json', import.meta.url), 'utf8'),
);
/** The shared rows as gathered attachments: an included row carries its exact fixture bytes. */
export const gathered = (): ExportAttachment[] =>
  structuredClone(fixture.input.attachments).map(
    ({ sha256: _sha, file, ...row }: Record<string, unknown> & { file?: string }) =>
      file === undefined
        ? row
        : {
            ...row,
            bytes: Uint8Array.from(Buffer.from(fixture.input.attachmentBytes[file], 'base64url')),
          },
  );
export const input = (): ExportView => {
  const {
    signingKeys,
    attachments: _rows,
    attachmentBytes: _bytes,
    ...rest
  } = structuredClone(fixture.input);
  return {
    ...rest,
    attachmentEntries: gathered(),
    statusWriters: Object.keys(signingKeys),
    signingKeys: Object.fromEntries(
      Object.entries(signingKeys as Record<string, string>).map(([writer, key]) => [
        writer,
        Uint8Array.from(Buffer.from(key, 'hex')),
      ]),
    ),
  };
};
