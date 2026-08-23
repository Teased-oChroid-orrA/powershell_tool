# PowerShell scripts (reference only)

These are the original PowerShell implementation this project was migrated
from. They are kept here for historical reference and as an independent
second implementation to diff behavior against if a discrepancy between the
two is ever suspected.

**The WinUI 3 application does not load, execute, shell out to, or otherwise
depend on anything in this folder, at build time or run time.** Every piece of
logic here (file walking, retry/timeout, encoding detection, DOCX/PPTX/RTF/PDF
extraction, matching modes, the incremental cache, HTML/CSV/JSON report
generation) has an equivalent, independently-tested C# implementation under
`src/TextInFilesSearch.Core/`.

If you don't have PowerShell available, or don't want it, these two files can
be deleted entirely with no effect on the application.
