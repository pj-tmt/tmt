/** A visual instance retains area context without creating a second agent identity. */
export type OfficeSelection =
  | { kind: 'area'; areaId: string }
  | { kind: 'agent'; identityId: string; areaId?: string };
