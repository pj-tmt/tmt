import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import { flushSync } from 'react-dom';
import { BrowserAction, BrowserDrawer, BrowserMenu, type BrowserMenuItem } from '../src/react.js';
import {
  mountBrowserMenu,
  type BrowserMenuHost,
  type BrowserMenuHostOptions,
} from '../src/menu-host.js';

export function NativeMenu({
  options,
  controls,
}: {
  options: BrowserMenuHostOptions;
  controls?: RefObject<BrowserMenuHost | null>;
}) {
  const root = useRef<HTMLDivElement>(null);
  const own = useRef<BrowserMenuHost | null>(null);
  const owner = controls ?? own;
  const latest = useRef(options);
  latest.current = options;
  useLayoutEffect(() => {
    const menu = mountBrowserMenu(root.current!, latest.current);
    owner.current = menu;
    return () => {
      menu.dispose();
      owner.current = null;
    };
  }, [owner]);
  useLayoutEffect(() => {
    owner.current?.update(options);
  }, [owner, options]);
  return <div ref={root} data-menu-host />;
}

/** Actual React and native renderers share one fixture model and exercise their real lifetimes. */
export function MenuHostFrame({ params }: { params: URLSearchParams }) {
  const [selected, setSelected] = useState('alpha');
  const [version, setVersion] = useState('A');
  const [result, setResult] = useState('none');
  const [transitions, setTransitions] = useState<string[]>([]);
  const [reverse, setReverse] = useState(false);
  const [removeAlpha, setRemoveAlpha] = useState(false);
  const [nativeMounted, setNativeMounted] = useState(true);
  const [reactMounted, setReactMounted] = useState(true);
  const [request, setRequest] = useState(0);
  const [drawer, setDrawer] = useState(false);
  const native = useRef<BrowserMenuHost | null>(null);
  const captured = useRef<Event | null>(null);
  useEffect(() => {
    const capture = (event: Event) => {
      if ((event.target as Element).closest?.('[role^="menuitem"]')) captured.current = event;
    };
    document.addEventListener('click', capture, true);
    return () => document.removeEventListener('click', capture, true);
  }, []);
  const receive = useCallback(
    (key: string, event: MouseEvent | ReactMouseEvent<HTMLButtonElement>) => {
      const source = event instanceof MouseEvent ? 'native' : 'react';
      const original = event instanceof MouseEvent ? event : event.nativeEvent;
      setSelected(key);
      setResult(`${source} ${version} ${key} original=${original === captured.current}`);
    },
    [version],
  );
  const visibility = useCallback(
    (source: string, open: boolean) => {
      if (!open && params.get('replace-on-close') === 'true') flushSync(() => setVersion('B'));
      setTransitions((values) => [...values, `${source}:${open}`]);
    },
    [params],
  );
  const items = useMemo<BrowserMenuItem[]>(() => {
    const rows: BrowserMenuItem[] = [
      { key: 'alpha', label: 'Alpha', checked: selected === 'alpha' },
      { key: 'blocked', label: 'Blocked', disabled: true, disabledReason: 'Read only' },
      { key: 'gamma', label: 'Gamma', checked: selected === 'gamma' },
      { key: 'delta', label: 'Delta', checked: selected === 'delta', separated: true },
    ].filter((item) => !removeAlpha || item.key !== 'alpha');
    return reverse ? rows.reverse() : rows;
  }, [selected, removeAlpha, reverse]);
  const nativeVisibility = useCallback((open: boolean) => visibility('native', open), [visibility]);
  const reactVisibility = useCallback((open: boolean) => visibility('react', open), [visibility]);
  const nativeOptions = useMemo<BrowserMenuHostOptions>(
    () => ({
      label: 'Native menu',
      items: items.map(({ content: _content, ...item }) => item),
      description: 'Choose a fixture value',
      triggerText: params.get('kind') === 'text' ? 'Choose' : undefined,
      onSelect: receive,
      onOpenChange: nativeVisibility,
    }),
    [items, params, receive, nativeVisibility],
  );
  return (
    <main className="gallery-main gallery-stack">
      <h1>Menu renderer parity</h1>
      <div className="gallery-row">
        {reactMounted && (
          <BrowserMenu
            label="React menu"
            items={items}
            description="Choose a fixture value"
            triggerText={params.get('kind') === 'text' ? 'Choose' : undefined}
            openRequest={request}
            onSelect={receive}
            onOpenChange={reactVisibility}
          />
        )}
        {nativeMounted && <NativeMenu options={nativeOptions} controls={native} />}
      </div>
      <p role="status" data-testid="selection">
        {result}
      </p>
      <p data-testid="transitions">{transitions.join(' ')}</p>
      <div className="gallery-row">
        <BrowserAction
          type="button"
          variant="secondary"
          label="Reverse items"
          onActivate={() => setReverse((value) => !value)}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Remove Alpha"
          onActivate={() => setRemoveAlpha(true)}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Set Gamma"
          onActivate={() => setSelected('gamma')}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Toggle native host"
          onActivate={() => setNativeMounted((value) => !value)}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Toggle React host"
          onActivate={() => setReactMounted((value) => !value)}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Open native request"
          onActivate={() => native.current?.open()}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Open React request"
          onActivate={() => setRequest((value) => value + 1)}
        />
        <BrowserAction
          type="button"
          variant="secondary"
          label="Open drawer"
          onActivate={() => setDrawer(true)}
        />
      </div>
      <BrowserDrawer
        open={drawer}
        title="Menu fixture"
        panel="menu-fixture"
        onDismiss={() => setDrawer(false)}
      >
        <NativeMenu options={{ ...nativeOptions, label: 'Drawer native menu' }} />
      </BrowserDrawer>
    </main>
  );
}
