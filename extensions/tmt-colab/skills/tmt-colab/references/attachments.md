# Document images

For a local page, attach the image first, then reference its attachment ID in HTML:

```sh
tmt colab attachment attach PAGE ./image.png --json
```

Read the returned attachment ID; write `<img src="colab-attachment:FULL-ID" alt="Description">`
through the normal page write. A unique eight-character ID prefix also works. Prefer the full ID
for durable references. Add width/height for stable layout and meaningful alt text; `alt=""` marks
a decorative image. Byte-derived PNG, JPEG and WebP display; other files remain downloads.

The host resolves only this page's verified document attachments through its admitted read path.
It supplies a checked raster display copy to the sandbox; author HTML gets no storage capability,
URL or network permission. Unknown, ambiguous, removed, inaccessible or unsupported references
show inert placeholders. A render reads at most 64 MiB in document order; later images show a
limit placeholder. The page's 2 MiB HTML cap still applies: do not embed large base64 images.
Keep `data:` images for tiny inline icons only.

Local reader links can display document images under the same page ticket and verified read
admission as Files. Ending access clears display copies. Epoch re-sealing replaces the attachment
ID; the old HTML reference then shows an unavailable placeholder. Reference repair is deferred
to #2746; this feature does not rewrite saved HTML.

This image path is available on the Local Remote object backend. The Firebase MVP has no file
blob resource yet: use text/non-image alternatives or tiny inline icons there, and expect
`Image not available here` placeholders rather than fetching around the backend. Future blobs
must follow page membership and epoch admission; this feature supplies no public object URL.
