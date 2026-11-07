# Windows Search Workbench package

The Windows build uploads `app-tui-win-x64.7z` using 7z format, LZMA, maximum compression, a 64 MiB dictionary, and solid blocks. The build tests archive integrity before upload.

A second artifact, `7za-extractor-win-x64`, contains the standalone x64 `7za.exe` and its license. The workflow obtains 7za from the official 7-Zip 26.05 Extra archive, verifies SHA-256 `f27a8f103c84785026edcba1f67faf1357225df9d19ddb0b7760d55947141bc7`, then tests it against the app archive.

Windows workflow run `42376424717` succeeded. It built and smoke-launched the dashboard, verified the LZMA archive with 7-Zip, and tested that the standalone `7za.exe` can read the archive. GitHub uploaded `app-tui-win-x64` (448,627,377 bytes) and `7za-extractor-win-x64` (1,340,214 bytes).

The pinned Extra archive also matched the official GitHub release asset digest locally. The run used commit `c03ef0ae2152c97a2fa3d6c30ae83f012545f0ac`; its app sources match current PR head, while the later commit only enables LFS in the qualification workflow.

GitHub Actions artifacts use the repository's configured retention period (default: 90 days); they are not permanent release attachments.
