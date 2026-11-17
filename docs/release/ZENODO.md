# Graphite demo on Zenodo

The owner published the three-file bundle at
[DOI 10.5281/zenodo.24282715](https://doi.org/10.5281/zenodo.24282715).
The app pins the version-specific ZIP URL in `demo/distribution.json`.
An independent download matches the 411,918,125-byte archive and SHA-256
below. Direct offline ZIP import remains available. The build command below
reproduces the existing bundle; future changes need a new reviewed version.

On 2026-11-16 the production native downloader also fetched this published
ZIP anonymously into a disposable profile and imported all 60 PDFs. Each
PDF matched its manifest size and SHA-256. This verifies the configured
download and import path; it does not certify new installed packages or
semantic model execution. The explicit network test is reproducible with:

```bash
GRAPHITE_TEST_ZENODO_DOWNLOAD=1 cargo test --manifest-path src-tauri/Cargo.toml --locked --offline published_zenodo_archive_downloads_and_imports -- --ignored
```

Cargo stays offline for dependency resolution; this opted-in test downloads
the public 412 MB archive over HTTPS. Ordinary tests never run it implicitly.

## Build the upload folder

From the project root, using the already verified public PDFs:

```bash
npm run demo:archive -- --zenodo --output artifacts/demo-zenodo-v2-r1
```

If PDFs are missing, obtain them explicitly first:

```bash
npm run demo:prepare -- --download
```

The archive command verifies each PDF, builds a deterministic ZIP, checks
its entries and refuses to overwrite different existing outputs. Choose a
new output folder after changing metadata or instructions. Keep earlier
verified archives and receipts intact.

Upload all three files from `artifacts/demo-zenodo-v2-r1/` to one Zenodo record:

| File                                         | Purpose                                                                                   |
| -------------------------------------------- | ----------------------------------------------------------------------------------------- |
| graphite-demo-seismology-ml-v2-r1.zip        | The 60 original PDFs, README, attribution, rights review, manifest and internal checksums |
| graphite-demo-seismology-ml-v2-r1.zip.sha256 | SHA-256 checksum of the complete ZIP                                                      |
| README.txt                                   | Setup and offline-import instructions, readable without opening the ZIP                   |

The build receipt is `artifacts/demo-zenodo-v2-r1.verification.json`, outside the
upload folder. It is local verification evidence, not an upload.
Only README.txt is duplicated outside the ZIP, byte-for-byte. The manifest,
attribution and rights evidence remain inside the ZIP. This is packaging
revision v2-r1 of the unchanged seismology-ml-v2 corpus. Graphite derives a
root collection and child collections from the curated themes.
Models, user databases, indexes and private documents are not included.

Current prepared ZIP:

```text
Bytes: 411918125
SHA-256: 8286328f700d7c3773587a86aed58e980c92d4ad7f61e9e10eb2da7b11f0fa28
```

## Describe the record

Suggested title: **Graphite demonstration corpus: 60 papers on seismology
and machine learning**. Record the collection version as `seismology-ml-v2`.
Use the owner's actual curator identity and the real Graphite repository
address. Do not invent creator details or copy an example repository URL.
The DOI for this record identifies the curated collection, not authorship
of the individual papers. Keep each article's original DOI and author credit.

Suggested description:

> A curated collection of 60 open-access papers for reproducible exploration
> of seismology and machine learning in Graphite. The archive contains
> unchanged original PDFs, bibliographic metadata, per-paper attribution,
> reuse-rights evidence and integrity checksums. Topics include detection,
> phase picking, classification, wave simulation and ML-enabling datasets.
> Graphite prepares document text and citation connections locally; optional
> search and answer models are installed separately. The collection is for
> demonstration and research exploration and is not a validated benchmark of
> answer quality or an endorsement of operational earthquake prediction.

The papers use their own licenses: one CC BY 2.0, two CC BY 3.0 and 57 CC BY
4.0. Describe this mixture in the record and retain the supplied license
links and credits. Graphite's GPL software license does not relicense the
papers. Restricted datasets mentioned in the articles are not included.

## Future record updates

1. Review the three files and record metadata, then publish the record using
   your own Zenodo account.
2. Obtain the direct ZIP download URL from that specific published record.
   Supply that URL for `demo/distribution.json`; use the version-specific
   file address, not an address that silently moves to the latest version.
3. Keep the configured archive filename, manifest ID, byte count and SHA-256
   equal to the verified bundle. Download the published ZIP independently
   and verify it against the supplied checksum.
4. Build Graphite with the configured URL and test **Prepare library for
   AI** in a fresh disposable profile. Compare it with importing the same
   ZIP through **Already downloaded?**. Both must produce the same verified
   papers and retain their source partition.
5. Publish the matching application only after these checks. A later corpus
   update needs its own reviewed archive, version-specific link and checksum;
   never replace bytes behind a previously advertised checksum.

Use the actual published record link on the project website and in release
notes. Installer publication remains separate from this dataset record.
