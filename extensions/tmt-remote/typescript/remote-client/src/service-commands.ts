/** Copy-only terminal references. Native grammar tests admit the same product catalog. */
import commands from '../../../rust/tmt-remote/assets/service-commands.json' with { type: 'json' };

export const serviceCommands = Object.freeze(commands);

/** Accept concrete native input spellings only; no shell syntax can enter a printed command. */
export function deploymentCommands(
  project: string,
  region: string,
): { preview: string; deploy: string } {
  if (!/^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(project))
    throw new Error('Enter a Firebase project ID: 6–30 lowercase letters, digits or hyphens.');
  if (!/^[a-z0-9-]{1,32}$/.test(region))
    throw new Error('Enter the database region from your Firebase project.');
  const fill = (template: string) =>
    template.replace('<project-id>', project).replace('<region>', region);
  return { preview: fill(commands.preview), deploy: fill(commands.deploy) };
}
