/** Read the existing selected database path without opening SQLite. */
export function readCoreDatabase(directory: string): string;
/** Refuse either database name (including dangling links) or unexpected sidecars. */
export function assertNoCoreDatabase(directory: string, message: string): void;
