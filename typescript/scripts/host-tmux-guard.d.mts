export interface HostTmuxViolation {
  readonly command: string;
  readonly reason: string;
}
type Environment = Readonly<Record<string, string | undefined>>;
export function resolveTmuxCommand(word: string): string | undefined;
export function simpleCommands(source: string): string[][];
export function hostTmuxViolations(
  commandLine: string,
  env?: Environment,
  uid?: number
): HostTmuxViolation[];
export function refusal(violations: readonly HostTmuxViolation[]): string;
export function decide(payload: unknown, env?: Environment, uid?: number): string | undefined;
