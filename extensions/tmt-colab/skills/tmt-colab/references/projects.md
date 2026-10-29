## Projects

Projects organize the owner's page list (a page can be in several); display-only, unsigned.
Commands: `tmt colab project add|remove PAGE NAME...`, `project create NAME`, `project ls`,
`project rename NAME NEW`, `project delete NAME` (rename merges onto an existing project;
rename and delete reach archived pages and never delete one). A name is 1 to 32 characters (no control characters, no edge or
doubled spaces); a page is in at most 8. `add` and `remove` replace the page's set (last
writer wins); `changed:false` means no change. `ls` and `show --json` list each page's
`projects`; `ls --project NAME` filters. `pin|unpin PAGE`: Home order (20 max).

The Colab side menu lists the projects the owner pins, in the owner's order (20 max): `project
pin NAME [--position N]` (a pinned project moves) and `project unpin NAME`. `project ls` gives each
project's `pinned` place (or null) and `pinsAutomatic`: until the owner pins one, the menu shows the
five most recently used projects, and the first pin keeps them and adds to them. These are not the
page pins above.

The page list also offers **Save as shortcut** (or **Edit shortcut** while viewing one) and
**Manage shortcuts**. A shortcut keeps the current project, text, creator, sort, stage-tab and
Needs me filters. Its editor also supports manual Page state and Include archived. Manual state
is independent of task stage; Include archived adds archived pages, while the Archived tab stays
archived-only. No selected project means all projects.

Shortcuts are saved in this browser on this machine, per space; they are not synced or shared.
Explicit pinned projects migrate once in their existing order; automatic recent projects do not.
Migration leaves the owner pins unchanged. Manage can create, rename, edit, delete and reorder
shortcuts, with a matching-page preview. Failed storage writes keep the draft and report the error.
The current pinned-project sidebar and Pin to sidebar controls remain available.
