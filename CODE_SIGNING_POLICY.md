# Code signing policy

Graphite is distributed without a trusted publisher signature. macOS releases
are not notarized. This is the chosen release policy, not a pending Apple
Developer enrollment. Publication still requires an explicit owner action
after native installation and acceptance checks pass.

The repository owner controls release approval, tags and publication.
Package metadata uses the neutral Graphite identity. No Apple account,
paid certificate or signing-service enrollment is required or configured.

## Candidate and production separation

The candidate workflow builds Linux x64 and macOS Apple Silicon packages
with read-only repository permissions and no signing secrets. It records
package checksums and a source revision. It cannot publish a release.

The release workflow reuses these checks and builds for an existing version
tag. Only its final draft job has repository write permission, using the
temporary GitHub Actions token. It attaches verified installers to an
unpublished draft and refuses to overwrite a published release. Public
publication remains an owner action; no personal access token is needed.

Any future production credentials belong only in an owner-managed
`production-release` environment with approval rules and a new explicit
owner decision. Never put certificates, tokens, account
identities, private keys, or temporary signing files in source, caches,
artifacts, or logs. Untrusted pull requests receive no release credentials.

## macOS

The public DMG uses a free local ad-hoc signature for Apple Silicon, with
`signingIdentity` set to `-`. It uses no Developer ID Application certificate
and no notarization. The build wrapper excludes inherited Apple credentials.
Ad-hoc signing establishes no Apple-verified publisher identity; public copy
calls these builds unsigned and unnotarized. Package metadata's `signed:
false` means no trusted publisher signature, even when an ad-hoc signature
is present. See the [Tauri signing guide](https://v2.tauri.app/distribute/sign/macos/).

Users may encounter a Gatekeeper warning. After checking the release source
and checksum, they can follow [Apple's app-specific opening instructions](https://support.apple.com/en-us/102445)
if they trust the download. Do not disable Gatekeeper globally or recommend
blanket quarantine removal. Test a browser-downloaded DMG on a clean Mac;
an automated runner launch does not establish this first-launch experience.

## Linux and Windows

Linux and macOS packages carry SHA-256 checksums. Checksums establish file
integrity only; they do not authenticate a publisher. Windows is currently
unavailable; we are working on support. See [Windows status](docs/release/WINDOWS.md).

## Incident response

Stop publication if artifacts or signing credentials are suspect. Preserve
audit evidence, revoke affected credentials through the issuing service,
and publish corrected artifacts and checksums after review. Restore a
compatible Graphite profile backup when rolling back a database version.
Do not alter Zotero data as part of release recovery.

See the [privacy policy](PRIVACY.md) and [build contract](docs/release/BUILD.md).
