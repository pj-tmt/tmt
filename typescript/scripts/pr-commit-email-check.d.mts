export type HostnameCommit = {
  sha: string;
  roles: { role: 'author' | 'committer'; email: string }[];
};
export type CommitEmailResult = { checked: number; base: string; findings: HostnameCommit[] };
export function isHostnameEmail(email: unknown): boolean;
export function hostnameCommits(log: string): HostnameCommit[];
export function checkCommitEmails(input: {
  event: unknown;
  reader: { git(args: string[]): string };
}): CommitEmailResult;
export function commitEmailReport(result: CommitEmailResult): string;
