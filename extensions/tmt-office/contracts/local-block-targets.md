# Retained local block storage

This is the migration source for [whole-world layouts](world-v1.md), not a public
editing interface. Local block CLI, private companion and HTTP operations have
been removed; use `tmt office layout show/apply` or the world HTTP endpoint.
Remote Firestore blocks retain their separate contract.

Schema 19 stores layouts by identity UUID or installation lobby. A saved empty
layout is an intentional override. An absent lobby uses the bundled
`lobby-preset-v1.json`; an absent identity block contributes no furniture.
World projection preserves stored object order, customization and resource
bindings, including retained temporary/retired identities. The world contract
owns projection, revision-zero fingerprint fencing and atomic row retirement.
Reading or upgrading the database does not replace existing layouts.
