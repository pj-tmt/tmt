import {
  useLayoutEffect,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';
import { browserUiClasses as c, type BrowserTab, type BrowserTabSelectReason } from './static.js';
export type { BrowserTab, BrowserTabSelectReason } from './static.js';
export interface BrowserTabsProps {
  label: string;
  tabs: readonly BrowserTab[];
  selected: string;
  onSelect(
    id: string,
    reason: BrowserTabSelectReason,
    event: KeyboardEvent<HTMLButtonElement> | MouseEvent<HTMLButtonElement>,
  ): void;
  status?: ReactNode;
  /** `boxed` (default) frames the strip and its panels; `flush` is a strip with an underline over
   * content that sits on the page, for a list a few views of one set share. */
  variant?: 'boxed' | 'flush';
  panelTabIndex?: 0 | -1;
  children(tab: BrowserTab): ReactNode;
}

/** Selection and panel lifetimes belong to the host. Every panel stays mounted. */
export function BrowserTabs({
  label,
  tabs,
  selected,
  onSelect,
  status,
  variant = 'boxed',
  panelTabIndex,
  children,
}: BrowserTabsProps) {
  const strip = useRef<HTMLDivElement>(null);
  const tabIds = JSON.stringify(tabs.map(({ id, tabId }) => [id, tabId]));
  useLayoutEffect(() => {
    const list = strip.current;
    const tab = list?.querySelector<HTMLElement>('[aria-selected="true"]');
    if (!list || !tab) return;
    // Move this strip only: scrollIntoView would also move the host's page.
    const keepVisible = () => {
      const bounds = list.getBoundingClientRect();
      const item = tab.getBoundingClientRect();
      if (item.left < bounds.left) list.scrollLeft += item.left - bounds.left;
      else if (item.right > bounds.right) list.scrollLeft += item.right - bounds.right;
    };
    keepVisible();
    const observer = new ResizeObserver(keepVisible);
    observer.observe(list);
    observer.observe(tab);
    return () => observer.disconnect();
  }, [selected, tabIds]);
  function key(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    let next: number;
    switch (event.key) {
      case 'ArrowLeft':
        next = (index + tabs.length - 1) % tabs.length;
        break;
      case 'ArrowRight':
        next = (index + 1) % tabs.length;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = tabs.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    const target = strip.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next];
    target?.focus({ preventScroll: true });
    onSelect(tabs[next].id, 'keyboard', event);
  }
  return (
    <div className={c.tabs} data-variant={variant}>
      <div className={c.tabsHeader}>
        <div className={c.tabList} role="tablist" aria-label={label} ref={strip}>
          {tabs.map((tab, index) => (
            <button
              key={tab.id}
              id={tab.tabId}
              type="button"
              className={c.tab}
              role="tab"
              aria-controls={tab.panelId}
              aria-selected={tab.id === selected}
              aria-label={
                tab.state ? `${tab.state.mark} ${tab.label} · ${tab.state.word}` : tab.label
              }
              tabIndex={tab.id === selected ? 0 : -1}
              onKeyDown={(event) => key(event, index)}
              onClick={(event) => onSelect(tab.id, 'activation', event)}
            >
              {tab.state && (
                <span className={c.tabMark} data-tone={tab.state.tone} aria-hidden="true">
                  {tab.state.mark}
                </span>
              )}
              <span className={c.tabLabel} aria-hidden="true">
                {tab.label}
              </span>
            </button>
          ))}
        </div>
        {status !== undefined && <div className={c.tabsStatus}>{status}</div>}
      </div>
      {tabs.map((tab) => (
        <div
          key={tab.id}
          id={tab.panelId}
          className={c.tabPanel}
          role="tabpanel"
          aria-labelledby={tab.tabId}
          tabIndex={panelTabIndex}
          hidden={tab.id !== selected}
        >
          {children(tab)}
        </div>
      ))}
    </div>
  );
}
