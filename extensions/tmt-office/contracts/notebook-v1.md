# Saved notebook resource v1

Implemented locally, not released. The spatial binding and browser view read the
owner-local Markdown used by `tmt notes path`; they are not another notes store,
editor, search engine or arbitrary file API.

The resource reference is a saved identity UUID. Names are display text. Opening
a notebook revalidates an active saved identity through the existing identity
and notes eligibility owners. Contractors and retired identities cannot read
through this resource; retirement still preserves their original Markdown file.
A same-name replacement has a different UUID and does not inherit the old file.
Removing or moving a notebook object never deletes, exports or retargets notes.

`GET /api/v1/local/notebooks/<uuid>` uses the existing loopback bearer authority.
It returns `{identityId,name,content}` from the installation's canonical notes
path. There is no write endpoint, caller-supplied path or implicit initialization.
Missing notes return `NOTEBOOK_NOT_FOUND`; creating notes remains an explicit
agent action through `tmt notes path --identity <name>`. Refresh rereads the file;
there is no background watcher, content cache or duplicated database record.

The read ceiling is 1 MiB of exact UTF-8. Oversized or malformed text fails without
truncation, replacement or file modification. This is a viewer limit, not a file
write restriction. Resolution pins directory handles below the configured data
root and refuses symlinks and nonregular files. Missing, ineligible and unavailable
results are distinct; errors disclose no filesystem paths or secret content.

The browser displays inert text without executing Markdown/HTML or fetching
embedded images/links. Loading or switching a notebook never dispatches work.
The data-only `notebook` binding uses `identityId` and the registered
`notebook.open` capability; admission is not proof of saved eligibility or access.
The editor must require an explicit saved-identity selection and retain resource
references if an identity retires. No automatic notebook placement or content
publication accompanies identity creation or personal-area assignment.

Acceptance: missing reads create no note files/directories; CLI-written exact
text is visible after refresh/restart; temporary/retired/same-name replacement
identities do not inherit access; final and parent symlinks, FIFO, directories,
invalid UTF-8 and oversized files fail without touching their contents. HTTP
auth/method/path rejection precedes storage/file access. Browser switches fence
late results, and desktop/narrow rendering exposes an explicit refresh and clear
read-only/error state. Layout/object removal preserves the source file.
