import { createContext, useContext, type RefObject } from 'react';

/** One shell-owned presentation, shared by the header and the cosmetic frame key request. */
export const QuickSwitcherContext = createContext<{
  open: boolean;
  activate(): boolean;
  control: RefObject<HTMLButtonElement | null>;
} | null>(null);

export const useQuickSwitcher = () => useContext(QuickSwitcherContext);
