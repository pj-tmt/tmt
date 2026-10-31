import { BrowserIconAction } from '@tmt/browser-ui/react';
import { Link } from '@tanstack/react-router';
import { Moon, Sun } from 'lucide-react';
import { useContext, useSyncExternalStore, type ReactNode, type Ref } from 'react';
import { ColabHeader } from './colab-header.js';
import { SharedTopBar } from './shared-top-bar.js';
import { text } from './strings.js';
import { getTheme, setThemeChoice, subscribeTheme } from './theme.js';

/** The header of a Colab route that is not a loaded page: its title in the shared bar's title slot,
 * or the module's own header outside it. */
export function AppHeader({
  linked = true,
  title = text.pages,
  titleVisible = true,
  crumb,
  remote,
  headerRef,
  headingSuffix,
  page = false,
}: {
  linked?: boolean;
  title?: string;
  titleVisible?: boolean;
  /** Route-owned ancestry before the current title in the shared bar. */
  crumb?: ReactNode;
  remote?: { deviceName?: string };
  headerRef?: Ref<HTMLElement>;
  headingSuffix?: ReactNode;
  /** The title names a page (a shell page), not a Colab section. */
  page?: boolean;
}) {
  const sharedBar = useContext(SharedTopBar);
  return (
    <ColabHeader
      title={title}
      titleVisible={titleVisible}
      crumb={crumb}
      page={page}
      headerRef={headerRef}
      remote={remote}
      headingSuffix={headingSuffix}
      home={
        linked
          ? (brand, name) => (
              <Link to="/" aria-label={name} title={name}>
                {brand}
              </Link>
            )
          : undefined
      }
      actions={sharedBar ? undefined : <ThemeButton />}
    />
  );
}
function ThemeButton() {
  const dark = useSyncExternalStore(subscribeTheme, getTheme) === 'dark';
  return (
    <span className="theme">
      <BrowserIconAction
        type="button"
        variant="text"
        label={text.theme}
        icon={dark ? <Moon /> : <Sun />}
        onActivate={() => {
          setThemeChoice(getTheme() === 'dark' ? 'light' : 'dark');
        }}
      />
    </span>
  );
}
