# Release privacy and security checks

## Boundaries

The installed app starts no HTTP server. Zotero requests use GET on the
loopback Local API. An external answer provider must be explicitly chosen,
resolve exclusively to loopback, and use an explicit port. Both clients
bypass system proxy settings and reject redirects. The optional API key is
kept in memory and excluded from saved provider settings. Managed inference
is in-process. User-requested model downloads contact their named upstream
sources; library text is not part of those requests.

External answer streams have bounded frame, transfer and content sizes and
stop at the completion marker. Oversized or malformed responses fail instead
of growing memory without a limit.

The development preview binds 127.0.0.1:1420. Pages is static: no accounts,
forms, uploaded files, analytics, remote fonts, or server-side application.
Only allowlisted assets and the public demo attribution enter its output.
A local preview must serve the built site directory, not the checkout.

## Boundary checks

Package inspection verifies architecture, native runtime dependencies,
licenses and required resources. It rejects packaged databases, models,
profiles, development resources and private configuration directories.
Recognized text resources up to 1 MiB are checked for private paths,
credential signatures and private-key material. Symlink destinations are
checked for build-machine identities. Failures report categories rather
than matched values.

Compiled binaries are not searched for arbitrary build identities or
credential-like strings. Precompiled dependencies may contain upstream
build paths or public cryptographic test vectors. These strings do not by
themselves establish that Graphite packaged private data or credentials.
Compiler path remapping remains best-effort hygiene; absence of all source
paths in distributed binaries is not a release requirement.

| Risk                                      | Likelihood                 | Impact                                   | Mitigation / evidence                                                                                      |
| ----------------------------------------- | -------------------------- | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Provider error echoes a key or paper text | Plausible                  | Private data retained in errors          | HTTP bodies withheld; health/generation regression test                                                    |
| Proxy settings forward loopback requests  | Configuration dependent    | Local data leaves the device             | Explicit no-proxy clients; tested with unusable proxy                                                      |
| Redirect leaves a local integration       | Plausible                  | Local request reaches unintended service | Both clients reject redirects; native tests                                                                |
| Source text contains instructions         | Expected in arbitrary PDFs | Ungrounded output                        | Separate system/user roles, explicit untrusted-evidence rule, bounded read-only tools, citation validation |
| Malformed or oversized PDF                | Expected                   | Parsing failure/resource load            | Native header/size checks, PDF.js parser, existing malformed/oversized regression                          |
| Workflow token or artifact exposure       | Configuration dependent    | Repository or private-file exposure      | Read-only build jobs, SHA-pinned actions, explicit artifact paths, owner-only deploy job                   |

The tests verify boundaries and failure handling. They do not prove immunity
to prompt injection or scientific correctness. A local external provider
may itself contact other services; Graphite cannot control that process.
Only configure a provider whose own data handling is acceptable.

## Dependency and license review

Recheck the locked production dependencies before each public release:

```bash
npm audit --omit=dev
cargo audit --file src-tauri/Cargo.lock
```

The Rust command requires an independently installed `cargo-audit` and a
current RustSec database. Record the reviewed lockfile and advisory revision.
These checks contact advisory services; they do not require library files or
credentials. A zero-finding result is a snapshot, not a guarantee. Review
warnings and applicability instead of suppressing them to obtain a green
result. See the [RustSec tool documentation](https://github.com/rustsec/rustsec/tree/main/cargo-audit).

The GTK/GLib dependency chain has a known
[VariantStrIter advisory](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).
The linked GTK 0.18 stack uses GLib 0.18; the advisory identifies fixes in
GLib 0.20 and later. Graphite does not directly call that iterator, but this
does not prove it is unreachable through dependencies. This framework caveat
requires review alongside any newer advisory results.

The locked build dependency chain also includes rand 0.7.3 and
[RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html).
The reported condition involves recursive random-number calls through a
custom logger; the dependency is reached through the phf_generator build
chain. Source-level applicability needs review rather than treating every
lockfile match as a demonstrated application exploit. Maintenance notices
also remain for fxhash, paste, proc-macro-error and unic crates. The native
dependency review is not a clean-advisory claim.

Preserve the applicable third-party licenses and notices, including the
bundled fonts, PDF libraries and GPU runtime. Corresponding source must match
the distributed GPL binaries. A lockfile license list is not an exact binary
bill of materials or a completed notice review; inspect the final package.

## Public source and artifact checks

The public demo retains scholarly author attribution, official source links,
license evidence and exact-PDF hashes. These credits do not authorize personal
paths, hostnames, addresses, credentials or private library data. Pages
publishes the reviewed metadata and app captures; PDFs are distributed through
the separate verified archive.

The ignore rules exclude local planning, agent state, reports, caches and
private data from a fresh repository. They do not remove files already
tracked in an older checkout. Review and stage only the intended public
source before running:

```bash
npm run check:publication
npm run check:privacy
npm run build
npm run site:build
node scripts/privacy-check.mjs dist dist-site
```

The publication check rejects tracked files excluded by the publication
rules. Do not force-add ignored material. The privacy check rejects embedded
Git-object backups and scans tracked files for the build directory, home
directory, hostname, common credential signatures and private-key material.
It reports categories without printing sensitive values. Generated site
output and packaged text resources receive separate checks within the
inspection boundary above. Run them for the exact outputs being published.

Automated checks supplement manual review. They cannot identify every secret
or personal detail, do not scan Git history, and do not classify scientific
author names. Review screenshots, metadata, binary contents and corresponding
source as part of [release validation](BUILD.md). Keep local QA receipts out
of the public source tree.

macOS uses local ad-hoc signing without a paid certificate or notarization.
A future trusted certificate needs a deliberate publisher decision because
its subject can disclose a legal identity. See the
[signing policy](../../CODE_SIGNING_POLICY.md).

## PDF copies and optional real-demo preparation

These additions open no network listener. PDF editing runs in a local worker
and saves separate application copies. Native export accepts bounded PDF
bytes and a user-selected destination, preserves registered originals, and
uses atomic replacement. No general filesystem permission is granted to the
webview. The CSP permits WebAssembly compilation without enabling general
JavaScript string evaluation.

PDF byte requests must carry the validated document revision. Missing or
stale revisions are rejected; reopen the document if its underlying file
changes.

The optional demo download uses the version-specific HTTPS Zenodo URL in
`demo/distribution.json`, bounds redirects to approved Zenodo URLs, and checks archive and
per-PDF lengths and SHA-256 hashes before saving app-owned copies. It sends
no library content or credentials. Offline ZIP import rejects unsafe paths,
unexpected entries and mismatched hashes. The separate publisher acquisition
tool uses the allowlisted source URLs in the manifest. A separate source/library
partition keeps the demo out of Zotero. Cancellation retains completed,
verified files; retries can repair damaged application cache files.

| Risk                                         | Likelihood                      | Impact                                 | Mitigation                                                                      |
| -------------------------------------------- | ------------------------------- | -------------------------------------- | ------------------------------------------------------------------------------- |
| PDF editor overwrites a source attachment    | Possible destination mistake    | Original changes                       | Separate copies; native export rejects registered sources; atomic writes        |
| Publisher URL changes or redirects           | Expected over time              | Wrong PDF or unintended host           | Fixed HTTPS host list, exact size/hash verification; offline archive fallback   |
| Archive path or PDF is substituted           | Possible local error            | Unintended file access or invalid data | Canonical child paths, fixed filenames, manifest hashes, size bounds            |
| Demo selection overwrites user library state | Possible integration regression | Context or derived data loss           | Independent partition, prior-source restoration and real-import regression test |

Run the browser and native regression checks for the affected boundaries.
Live download availability can change; verify publisher acquisition and native
offline import separately. Parsing untrusted PDFs remains dependent on the
maintained PDF libraries. Passing these checks is not a guarantee that no
security defects remain.
