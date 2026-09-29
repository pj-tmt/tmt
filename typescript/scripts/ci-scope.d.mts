export interface CiAreas {
  readonly native: boolean;
  readonly office: boolean;
  readonly nativeOffice: boolean;
}

export function selectCiAreas(paths: readonly string[]): CiAreas;
export function readChangedCiAreas(base: string, head: string, cwd: string): CiAreas;
export function ciGatePasses(selected: string, results: readonly string[]): boolean;
export const NATIVE_OFFICE_UNREACHABLE: Readonly<
  Record<'tmt-adapters' | 'tmt-core', readonly string[]>
>;
