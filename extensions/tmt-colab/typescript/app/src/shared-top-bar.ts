import { createContext } from 'react';

/** True where the shell has installed shared top-bar controls, whether owned by Colab or its host, so its routes put their actions in that bar and draw no Search,
 * Theme or Remote control of their own. */
export const SharedTopBar = createContext(false);
