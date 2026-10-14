import { createContext, useContext, useState, type ReactNode } from 'react';

const PortalContainer = createContext<HTMLElement | null | undefined>(undefined);
export type UiThemeMode = 'light' | 'dark' | 'system';

/** A local variable/utility namespace. Never sets document or body theme. */
export function UiTheme({
  theme = 'system',
  children,
}: {
  theme?: UiThemeMode;
  children: ReactNode;
}) {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  return (
    <div data-tmt-ui="" data-tmt-theme={theme}>
      <PortalContainer.Provider value={container}>
        {children}
        <div data-tmt-ui-portal="" ref={setContainer} />
      </PortalContainer.Provider>
    </div>
  );
}

/** Pass this to Radix Portal.container; wait for non-null, never fall back to body. */
export function useUiPortalContainer(): HTMLElement | null {
  const container = useContext(PortalContainer);
  if (container === undefined) throw new Error('UI portals require an originating UiTheme');
  return container;
}
