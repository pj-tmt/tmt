## Projects

Projects organize the owner's page list (a page can be in several); display-only, unsigned.
Commands: `tmt colab project add|remove PAGE NAME...`, `project create NAME`, `project ls`,
`project rename NAME NEW`, `project delete NAME` (rename merges onto an existing project;
rename and delete reach archived pages and never delete one). A name is 1 to 32 characters (no control characters, no edge or
doubled spaces); a page is in at most 8. `add` and `remove` replace the page's set (last
writer wins); `changed:false` means no change. `ls` and `show --json` list each page's
`projects`; `ls --project NAME` filters. `pin|unpin PAGE`: Home order (20 max).

The legacy owner project pin list remains available through `project pin NAME [--position N]`,
`project unpin NAME` and `project ls`; these are separate from page pins. It no longer controls
the browser sidebar.

The page list also offers **Save as shortcut** (or **Edit shortcut** while viewing one) and
**Manage shortcuts**. A shortcut keeps the current project, text, creator, sort, stage-tab and
Needs me filters. Its editor also supports manual Page state and Include archived. Manual state
is independent of task stage; Include archived adds archived pages, while the Archived tab stays
archived-only. No selected project means all projects.

Shortcuts are saved in this browser on this machine, per space; they are not synced or shared.
The sidebar's Shortcuts group shows each saved filter and its matching-page count when the page
list is available. It offers Manage, New, Edit, Delete and keyboard or pointer reordering. On the
first successful local migration after the legacy pins arrive, explicit pins become project-only
shortcuts in order; automatic recent projects are excluded and owner pins are left unchanged.
Manage can create, rename, edit, delete and reorder shortcuts, with a matching-page preview. Failed
storage writes keep the draft and report the error.
