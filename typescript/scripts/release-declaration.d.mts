import type { ComponentMap } from './ci-scope.mjs';
export interface DeclarationRecord {
  schema: number;
  cut: string;
  product: string;
  cli: { tag: string; version: string; sha: string };
}
export interface DeclarationRelease { tag_name: string; draft: boolean; body?: string }
export type ObserveCli = (release: DeclarationRelease, floor: string) => { sha: string; status: string };
export interface DeclarationContext {
  map: ComponentMap;
  product: string;
  cut: string;
  releases: DeclarationRelease[];
  observeCli: ObserveCli;
}
export const DECLARATION_SUPPORT: string;
export const MESSAGE_PROTOCOLS_SUPPORT: string;
export const DECLARATION_FILE: string;
export const DECLARATION_OUTPUT: string;
export function declaresMinimumCli(product: string): boolean;
export function declarationAncestry(input: { repository: string }): ObserveCli;
export function gitDeclarationAncestry(git: (args: string[]) => string): ObserveCli;
export function declarationSource(cut: string, root?: string): ComponentMap;
export function resolveDeclaration(input: DeclarationContext): DeclarationRecord | null;
export function declarationMarker(record: DeclarationRecord | null): string;
export function readDeclarationMarker(body?: string): DeclarationRecord | null;
export function verifyDeclaration(input: DeclarationContext & { record: DeclarationRecord | null }): DeclarationRecord | null;
export function declarationBytes(input: { record: DeclarationRecord; map: ComponentMap; product: string; cut: string }): Buffer;
export function writeDeclaration(root: string, snapshot: { declaration?: DeclarationRecord | null; hashes: Record<string, string>; product: string; cut: string }, map: ComponentMap): void;
export function verifyDeclarationBytes(file: string, snapshot: { declaration?: DeclarationRecord | null; product: string; cut: string }, map: ComponentMap): void;

export function readDeclarationReleases(input: { record: DeclarationRecord | null; map: ComponentMap; product: string; cut: string; readRelease: (tag: string) => DeclarationRelease | null }): DeclarationRelease[];
export function captureDeclaration(input: { map: ComponentMap; product: string; cut: string; git: (args: string[]) => string; readRelease: (tag: string) => DeclarationRelease | null }): DeclarationRecord | null;
