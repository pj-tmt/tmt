import { createPortal } from 'react-dom';
import { getRouteApi, useRouter } from '@tanstack/react-router';
import { useRef } from 'react';
import type { Selection } from './management.js';
import { ShareDialog } from './share-dialog.js';

const rootRoute = getRouteApi('__root__');

export function ManagePage({
  pageId,
  title,
  open,
  close,
  changed,
}: {
  pageId: string;
  title: string;
  open: boolean;
  close(): void;
  changed?(operation: Selection['operation']): void;
}) {
  const port = rootRoute.useRouteContext().transport.management;
  const router = useRouter();
  const touched = useRef(false);
  return open && port
    ? createPortal(
        <ShareDialog
          port={port}
          pageId={pageId}
          title={title}
          committed={(operation) => {
            touched.current = true;
            changed?.(operation);
          }}
          close={() => {
            close();
            if (touched.current) {
              touched.current = false;
              void router.invalidate();
            }
          }}
        />,
        document.body,
      )
    : null;
}
