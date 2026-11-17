# Windows support status

Windows is currently unavailable. We are working on support; no release
date or supported installer is announced.

The current release targets Linux x64 and macOS Apple Silicon. See
[installation and verification](BUILD.md) for their requirements.

The repository retains Windows-specific configuration and a native compile
lane for development. A successful compile does not establish installer,
WebView2, local-model, PDF, upgrade or uninstall compatibility. Windows will
be added to the release matrix only after those workflows are validated on
a clean native system and the distribution policy is settled.
