import {
  Button,
  Command,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  UiTheme,
} from '@tmt/browser-ui/ui';
import '@tmt/browser-ui/ui.css';
import { FileText, Plus } from 'lucide-react';
import { useRef, useState, type ReactNode } from 'react';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';
import type { AttachRefusal } from './work-tree.js';

export type AddChoice =
  | { kind: 'attach'; pageId: string; title: string }
  | { kind: 'create'; title: string };

const t = text.workBand;
/** A task title is at most 120 code points, and the created page takes its title. */
const TITLE_POINTS = 120;

function why(refusal: AttachRefusal): string {
  if (refusal.kind === 'self') return t.refusedSelf;
  if (refusal.kind === 'loop') return t.refusedLoop;
  return t.refusedAttached(refusal.parentTitle);
}

/**
 * The one Add window (#5820 Mock 5): search the owner list and attach a page, or create a page with
 * the typed name when no page has exactly that title. A page that cannot be attached stays listed,
 * dimmed, with why. Enter picks the highlighted row; Esc closes. The owner's admission decides.
 */
export function AddSubpageDialog({
  title,
  pages,
  refusal,
  onChoose,
  onClose,
}: {
  /** This page's title. */
  title: string;
  pages?: readonly PageSummary[];
  refusal(pageId: string): AttachRefusal | undefined;
  /** Resolves undefined once attached, or the sentence to show. */
  onChoose(choice: AddChoice): Promise<string | undefined>;
  onClose(): void;
}): ReactNode {
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const content = useRef<HTMLDivElement>(null);
  const typed = query.trim();
  const needle = typed.toLocaleLowerCase();
  const candidates = (pages ?? []).filter(
    (page) =>
      !page.archived &&
      page.pageType !== 'shell' &&
      (needle === '' || page.title.toLocaleLowerCase().includes(needle)),
  );
  const exact = candidates.some((page) => page.title.trim() === typed);
  const choose = (choice: AddChoice) => {
    if (busy) return;
    setBusy(true);
    setError(undefined);
    void onChoose(choice).then((failure) => {
      setBusy(false);
      if (failure === undefined) onClose();
      else setError(failure);
    });
  };
  return (
    <UiTheme>
      <Dialog
        open
        onOpenChange={(open) => {
          if (!open && !busy) onClose();
        }}
      >
        <DialogContent
          ref={content}
          className="work-add-dialog"
          initialFocus={() =>
            content.current?.querySelector<HTMLInputElement>('[cmdk-input]') ?? null
          }
        >
          <DialogHeader>
            <DialogTitle>{t.addTitle(title)}</DialogTitle>
            <DialogDescription>{t.addDescription}</DialogDescription>
          </DialogHeader>
          <Command shouldFilter={false} className="work-add-command">
            <CommandInput
              value={query}
              onValueChange={(value) => {
                setQuery(value);
                setError(undefined);
              }}
              placeholder={t.search}
              maxLength={TITLE_POINTS}
              disabled={busy}
            />
            <CommandList>
              {typed !== '' && !exact && (
                <>
                  <CommandGroup>
                    <CommandItem
                      value={`create:${typed}`}
                      disabled={busy}
                      onSelect={() => choose({ kind: 'create', title: typed })}
                    >
                      <Plus aria-hidden="true" />
                      {t.create(typed)}
                    </CommandItem>
                  </CommandGroup>
                  <CommandSeparator />
                </>
              )}
              <CommandGroup heading={t.pages}>
                {candidates.map((page) => {
                  const refused = refusal(page.id);
                  return (
                    <CommandItem
                      key={page.id}
                      value={page.id}
                      disabled={busy || refused !== undefined}
                      data-refused={refused !== undefined}
                      onSelect={() =>
                        choose({ kind: 'attach', pageId: page.id, title: page.title })
                      }
                    >
                      <FileText aria-hidden="true" />
                      <span className="work-add-title">{page.title}</span>
                      {refused && <span className="work-add-why">{why(refused)}</span>}
                    </CommandItem>
                  );
                })}
                {pages && candidates.length === 0 && <p className="work-add-note">{t.noMatch}</p>}
                {!pages && <p className="work-add-note">{t.pageListUnavailable}</p>}
              </CommandGroup>
            </CommandList>
          </Command>
          {error && (
            <p className="work-add-error" role="alert">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={busy} onClick={onClose}>
              {t.cancel}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </UiTheme>
  );
}
