## Templates

A template is a starting page: TMT's own (`starter`, `report`, `review`, `compare`, `board`,
`gallery`) or a page of this space that someone saved. `tmt colab template ls` lists both and marks
the default. A create that names no template, layout or file copies the space default (`starter`
until `template default` says otherwise).

Before you write a page from a template, read its rules:

```sh
tmt colab template show NAME
tmt colab page create --title "Week 43" --template NAME
```

`show` numbers the rules: the HTML comments of the template's source, in order. Fill each block
as its rule says, keep the structure and the section order, and keep the comments (they are not
rendered). Do not add sections the rules do not name. `page read PAGE` prints a user template's
full source.

The new page is a copy of the template as it is now, then independent: editing the template later
changes no page made from it. Question blocks in the copy already name you and are already
registered and open; do not register them again, and answer them as you would any question
([questions](questions.md)).

To make a template of a page you wrote: `tmt colab template save PAGE --name NAME [--description
TEXT]`. The name is 1 to 40 of `a-z`, `0-9` and `-`, not one of TMT's. Write rules as one HTML
comment before each block, one line each. The page needs a layout and registered questions.
`template default NAME|ID` and `--reset` choose what a bare create copies; `template remove
NAME|ID` only stops the page being a template. Errors name the fix (`hint:`).
