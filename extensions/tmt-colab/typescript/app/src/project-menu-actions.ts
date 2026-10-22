import { createContext } from 'react';

/** The page list's project dialogs, offered to the shell's module menu while the list is shown
 * and the space can be managed. The list registers them; `undefined` withdraws them. */
export interface ProjectMenuActions {
  newProject(): void;
  manage(): void;
}
export const ProjectMenuActionsSlot = createContext<
  ((actions: ProjectMenuActions | undefined) => void) | null
>(null);
