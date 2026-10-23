import { BrowserAction } from '@tmt/browser-ui/react';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  classifyPageLink,
  type PageLinkRequest,
  type PageLinkTarget,
} from './page-link-request.js';
import { text } from './strings.js';
import './page-link-bar.css';

interface PendingLink {
  request: PageLinkRequest;
  target: PageLinkTarget;
}
interface LinkContext {
  base(): string;
  space?: string;
  navigate?(route: string, anchor?: string): void;
  inset(): number;
}

/** The parent owns disclosure and navigation. Frame claims never authenticate a gesture. */
export function usePageLinks(context: LinkContext) {
  const latest = useRef(context);
  latest.current = context;
  const [pending, setPending] = useState<PendingLink>();
  const held = useRef<PendingLink | undefined>(undefined);
  const used = useRef<string | undefined>(undefined);
  const owner = useMemo(() => {
    const reset = () => {
      held.current = undefined;
      setPending(undefined);
    };
    const open = (value: PendingLink) => {
      if (value.target.kind === 'refused' || used.current === value.request.renderId) return;
      reset();
      if (value.target.kind === 'internal' && !value.request.newTab) {
        used.current = value.request.renderId;
        latest.current.navigate?.(value.target.route, value.target.anchor);
        return;
      }
      // noopener returns null on success too: never infer a blocked popup from that result.
      window.open(value.target.href, '_blank', 'noopener,noreferrer');
    };
    return {
      reset,
      request(request: PageLinkRequest, active: boolean) {
        if (held.current || used.current === request.renderId) return;
        const target = classifyPageLink(request.href, latest.current.base(), latest.current.space);
        const value = { request, target };
        if (target.kind === 'internal' && active && latest.current.navigate) open(value);
        else {
          held.current = value;
          setPending(value);
        }
      },
      confirm(value: PendingLink) {
        if (held.current === value) open(value);
      },
      cancel(value: PendingLink) {
        if (held.current === value) reset();
      },
    };
  }, []);
  useEffect(
    () => () => {
      held.current = undefined;
    },
    [],
  );
  return {
    ...owner,
    bar: pending ? (
      <PageLinkBar
        key={pending.request.renderId}
        pending={pending}
        inset={() => latest.current.inset()}
        cancel={() => owner.cancel(pending)}
        open={() => owner.confirm(pending)}
      />
    ) : null,
  };
}

function PageLinkBar({
  pending,
  inset,
  cancel,
  open,
}: {
  pending: PendingLink;
  inset(): number;
  cancel(): void;
  open(): void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const currentInset = useRef(inset);
  currentInset.current = inset;
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const target = pending.target;
  const refused = target.kind === 'refused';
  const internal = target.kind === 'internal';
  const href = refused ? '' : target.href;
  const title = refused
    ? text.pageLink.refused
    : internal
      ? text.pageLink.internal
      : text.pageLink.external;
  useEffect(() => {
    const node = root.current!;
    const place = () => {
      node.style.top = `${Math.max(0, currentInset.current())}px`;
    };
    place();
    node.showPopover();
    node.querySelector<HTMLButtonElement>('button')?.focus();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
      if (node.matches(':popover-open')) node.hidePopover();
    };
  }, []);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);
  const url = !refused && !internal ? new URL(href) : undefined;
  return (
    <div
      ref={root}
      popover="auto"
      role="dialog"
      aria-label={title}
      className="page-link-bar"
      onToggle={(event) => {
        if (event.newState === 'closed') cancel();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.stopPropagation();
          event.preventDefault();
          cancel();
        }
      }}
    >
      {!refused && (
        <small>{internal ? text.pageLink.internalLabel : text.pageLink.externalLabel}</small>
      )}
      <h2>{title}</h2>
      {!refused && (
        <p className="page-link-url" dir="ltr">
          {internal ? (
            target.label
          ) : (
            <>
              <span>{url!.origin}</span>
              {href.slice(url!.origin.length)}
            </>
          )}
        </p>
      )}
      {!refused && <p className="page-link-hint">{text.pageLink.copyHint}</p>}
      {copyFailed && <p role="status">{text.pageLink.copyFailed}</p>}
      <div className="page-link-actions">
        {!refused && (
          <BrowserAction
            type="button"
            variant="primary"
            label={text.pageLink.open}
            onActivate={(event) => {
              if (event.isTrusted) open();
            }}
          />
        )}
        {!refused && (
          <BrowserAction
            type="button"
            variant="text"
            label={copied ? text.pageLink.copied : text.pageLink.copy}
            onActivate={(event) => {
              if (!event.isTrusted) return;
              void navigator.clipboard.writeText(href).then(
                () => {
                  setCopied(true);
                  setCopyFailed(false);
                },
                () => setCopyFailed(true),
              );
            }}
          />
        )}
        <BrowserAction
          type="button"
          variant={refused ? 'secondary' : 'text'}
          label={refused ? text.pageLink.close : text.pageLink.cancel}
          onActivate={(event) => {
            if (event.isTrusted) cancel();
          }}
        />
      </div>
    </div>
  );
}
