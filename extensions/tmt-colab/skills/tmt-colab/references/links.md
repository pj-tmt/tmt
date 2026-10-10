Annotation/Chat links use `/p/SHORT`. Every CLI page argument accepts a full UUID
or a lowercase UUID-shaped prefix of at least eight characters. The CLI resolves
it against the verified owner catalog, including retained deleted IDs, and returns
full `pageId` values. If `COLAB_PAGE_AMBIGUOUS` lists candidates, ask which page is
meant; if deleted or missing, report that instead of guessing a different page.
Older links use `#space=SPACE&path=%2Fpages%2FPAGE`. Decode the `path` fragment
value; the page ID follows `/pages/`. If the requested work needs the page, read
it through `tmt colab page read`; retain its revision for changes.
