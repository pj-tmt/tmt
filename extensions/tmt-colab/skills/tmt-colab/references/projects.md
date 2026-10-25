## Projects

Projects organize the owner's page list (a page can be in several); display-only, unsigned.
Commands: `tmt colab project add|remove PAGE NAME...`, `project create NAME`, `project ls`,
`project rename NAME NEW`, `project delete NAME` (rename merges onto an existing project;
rename and delete reach archived pages and never delete one). A name is 1 to 32 characters (no control characters, no edge or
doubled spaces); a page is in at most 8. `add` and `remove` replace the page's set (last
writer wins); `changed:false` means no change. `ls` and `show --json` list each page's
`projects`; `ls --project NAME` filters. `pin|unpin PAGE`: Home order (20 max).
