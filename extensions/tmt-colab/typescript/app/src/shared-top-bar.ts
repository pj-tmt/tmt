import { createContext } from 'react';

/** True where Colab is the root of the shell it sits in (no host slot): it installs the shared top
 * bar's common controls itself, so its routes put their actions in that bar and draw no Search,
 * Theme or Remote control of their own. */
export const SharedTopBar = createContext(false);
