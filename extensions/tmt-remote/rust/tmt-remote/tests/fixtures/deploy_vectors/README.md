# Offline composition fixture

`remote-envelope.json` contains the exact existing Remote-authored `../rules/colab.json` and
`../rules/colab.rules` test bytes. The fixture name in those inputs does not establish Colab
provenance; this is not a production declaration or a substitute for Colab's vector.

From `rust/`, regenerate into staging with:

```sh
CARGO_BUILD_JOBS=2 cargo run --offline --locked -p tmt-remote --example compose_firestore -- ../extensions/tmt-remote/rust/tmt-remote/tests/fixtures/deploy_vectors/remote-envelope.json /private/tmp/remote-vector-staging
```

Review the three outputs, then copy them here with the `remote-envelope.` prefix. The plan
pins both exact input digests even when a source change leaves composed Rules/indexes unchanged.
The mirror uses Colab's own vector directory read-only and reports pending until that vector
exists; a present vector without any member of its golden trio fails.
