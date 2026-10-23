export interface HarnessPins {
  schemaVersion: 1;
  versions: { claude: string; codex: string };
  packages: {
    name: string;
    version: string;
    installDirectory: string;
    url: string;
    integrity: string;
  }[];
}
export function downloadHarnessArchive(
  item: { url: string; integrity: string },
  archive: string,
  options?: { fetchImpl?: typeof fetch; maxBytes?: number }
): Promise<{ archiveBytes: number }>;
export function validateHarnessPins(pins: unknown): HarnessPins;
