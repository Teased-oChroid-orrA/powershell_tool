# Windows Search Workbench package

The Windows build uploads `app-tui-win-x64.7z` using 7z format, LZMA, maximum compression, a 64 MiB dictionary, and solid blocks. The build tests archive integrity before upload.

A second artifact, `7za-extractor-win-x64`, contains the standalone x64 `7za.exe` and its license. The workflow obtains 7za from the official 7-Zip 26.05 Extra archive, verifies SHA-256 `f27a8f103c84785026edcba1f67faf1357225df9d19ddb0b7760d55947141bc7`, then tests it against the app archive.

Local evidence: the pinned Extra archive downloaded and matched the published GitHub release asset digest. Windows execution and uploaded-artifact checks remain pending until the workflow runs.

GitHub Actions artifacts use the repository's configured retention period (default: 90 days); they are not permanent release attachments.
