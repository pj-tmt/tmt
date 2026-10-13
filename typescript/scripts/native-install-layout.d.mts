export const CLI_NAMESPACE: 'lib/tmt';
export const LEGACY_CLI_NAMESPACE: 'lib/tmux-team';
export interface CliLayout {
  prefix: string;
  namespace: typeof CLI_NAMESPACE | typeof LEGACY_CLI_NAMESPACE;
  root: string;
  current: string;
  release: string;
  executable: string;
  receipt: Record<string, unknown>;
}
export function readCliLayout(
  installationPrefix: string,
  inspectReceiptText?: (text: string) => string
): CliLayout;
