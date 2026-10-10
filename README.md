# release-index

Machine-written release index for tmt. Only the Release App advances this branch,
without force pushes or deletion; see `contracts/release-index-v1.md` on `main`.

- `channels/<product>/<alpha|stable>.json`: the newest verified release per channel.
- `records/<tag>.json`: immutable bootstrap release records.

Do not edit by hand.
