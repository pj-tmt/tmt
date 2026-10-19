import { createContext } from 'react';

/** The element a route renders into inside the app header's heading, so a page of the app can place
 * its own control there while it keeps the data the control needs. Null until the header mounts. */
export const HeaderSlot = createContext<HTMLElement | null>(null);
