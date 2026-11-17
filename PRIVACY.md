# Privacy policy

Graphite is a standalone, local-first desktop application. It requires no
Graphite account, has no telemetry, and does not send a research library,
documents, conversations, embeddings, indexes, or model prompts to a
Graphite-operated service.

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or
operating it.

## Network behavior

Graphite makes network connections only for explicit local integrations or
user-requested downloads:

- Zotero integration reads the Zotero Local API on the loopback interface.
  Redirects are rejected and system proxy settings are bypassed.
- Installing a semantic or managed agent model downloads the selected,
  version-pinned model artifacts after the user chooses the installation
  action. Research content is not sent with those download requests.
- Preparing the optional demo downloads its version-specific public Zenodo
  archive after confirmation. Graphite verifies the archive and every PDF
  against bundled checksums. Offline ZIP import needs no network connection.
- The optional external model provider accepts only an explicitly configured
  loopback endpoint. Non-loopback endpoints and redirects are rejected.
  System proxies are bypassed. Provider HTTP error bodies are withheld from
  app errors because they can echo credentials or private request content.

After their required model assets have been installed, local indexing,
retrieval, and managed-model generation run on the user's machine. Graphite
does not provide a cloud fallback.

## Local data

Graphite stores application-owned state under its operating-system application
data directory. This may include normalized Zotero metadata, extracted text,
indexes, embeddings, conversations, reader state, and downloaded model files.
Zotero remains the bibliographic source of truth, and Graphite does not write
to Zotero's database or attachment storage.

Uninstalling the application may intentionally leave application data in place
so that reinstalling or upgrading does not destroy research state. Users should
back up valuable Graphite state before manual cleanup or downgrade operations.

## Build and release infrastructure

GitHub release infrastructure operates on generated application packages and
release metadata. It is not an application runtime service and receives no
private library data. macOS packages use a local ad-hoc signature without
Apple accounts, paid certificates or notarization uploads.
