import { createContext, useContext, type ComponentProps, type ReactNode } from 'react';
import { NeedsYouBar } from './components/needs-you-bar.js';

/** Chapter-only cosmetic navigation; the existing card remains the sole action owner. */
export const PassageNavigation = createContext<ReactNode>(undefined);

export function PassageBar({
  complete = false,
  ...props
}: ComponentProps<typeof NeedsYouBar> & { complete?: boolean }) {
  const next = useContext(PassageNavigation);
  return (
    <NeedsYouBar
      {...props}
      actions={
        complete && next ? (
          <>
            {props.actions}
            {next}
          </>
        ) : (
          props.actions
        )
      }
    />
  );
}
