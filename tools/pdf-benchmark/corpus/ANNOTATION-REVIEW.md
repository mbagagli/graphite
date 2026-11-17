# Benchmark anchor review

The ten `paper01.json` through `paper10.json` files contain reviewed anchors
and candidate anchors suggested from manual source inspection. The runner
validates the files but refuses to score while any anchor has:

```json
"verificationStatus": "suggested-unverified"
```

Review each anchor against the source PDF at the listed one-based page. Then
make one of these two edits:

```json
"verificationStatus": "human-verified",
"verificationNote": "Verified by <reviewer> on <date>; source page and text confirmed."
```

or:

```json
"verificationStatus": "human-rejected",
"verificationNote": "Rejected by <reviewer> on <date>; <short reason>."
```

Correct the page, matcher, required flag, or relationship before marking an
anchor verified. Reject a suggestion that is ambiguous, visually absent, or
not scientifically useful. For a reading-order pair, verify or reject both
members together.

After editing an annotation file, update its `anchorsSha256` value in the
corresponding manifest, then run:

```text
graphite-pdf-benchmark-runner validate \
  tools/pdf-benchmark/corpus/smoke-manifest.json
graphite-pdf-benchmark-runner validate \
  tools/pdf-benchmark/corpus/representative-manifest.json
```

Validation permits unverified suggestions so annotation work can continue.
The separate `score` command remains blocked until every suggestion has been
verified or rejected. Rejected anchors never enter fidelity metrics.

The representative expansion uses `paper11.json` through `paper30.json` and
`representative-manifest.json`. Each new file contains five
`suggested-unverified` anchors inspected from the corresponding local PDF.
Review them with the same procedure, then update the changed `anchorsSha256`
values in the representative manifest. Do not run expanded extraction or
scoring until every suggestion has been verified or rejected.

The smoke and representative manifests remain separate. Do not copy, reorder,
or reinterpret the original smoke anchors while reviewing the new subset.

## Suggested `paper07` empty-page metadata

The following visual classifications require human confirmation:

- Physical page 2 appears to be an intentionally blank front-matter leaf.
- Physical page 129 appears to be an intentionally blank end leaf after the
  bibliography.
- Physical page 130 is a solid-colour back-cover graphic with no visible text.
  It appears image-only by design rather than a scanned text page.

None of these pages currently appears to require OCR. The manifest therefore
keeps `expectedOcrPages` empty unless human review finds visible text that the
native layer omitted.
