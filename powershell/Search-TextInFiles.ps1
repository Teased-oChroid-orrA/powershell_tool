<#
.SYNOPSIS
    Recursively searches a folder for text/document files containing one or more
    keyword filters, and produces a single self-contained HTML report - with
    optional parallel processing, incremental caching, and a long list of
    speed/robustness/UX improvements layered on top of the original tool.

.DESCRIPTION
    SAFETY MODEL (read this before running):
      - This script NEVER deletes, moves, renames, or edits any file under -SearchPath.
        Every file under -SearchPath is only ever opened for READING.
      - The ONLY files this script writes are: the HTML report; a .csv/.json export
        if you pass -ExportCsv/-ExportJson; and a small cache file if you pass
        -CacheFile - all under locations you specify. Nothing else on disk is touched.
      - No admin/elevated rights are required. No network/internet access is used
        anywhere (no Invoke-WebRequest, no module installs, no CDN scripts in the
        report - everything, including the expand/collapse UI, is native HTML5
        with zero JavaScript).
      - -Parallel requires PowerShell 7+. On Windows PowerShell 5.1 it's silently
        ignored (with a note) and the script runs sequentially, which still works
        fully - parallelism is a speed optimization, not a requirement.

    WHAT'S NEW IN THIS VERSION (on top of matching, grouping, exports, etc. from
    before):
      SPEED
      - -Parallel / -ThrottleLimit: process multiple files at once (PS7+).
      - A combined "does this line contain ANY filter at all" pre-check regex
        runs before the slower per-filter loop that figures out exactly which
        filter(s) matched - most lines never match anything, so this skips the
        expensive path for the overwhelming majority of lines.
      - Filters are now compiled once (regex mode) instead of rebuilt per line.
      - Large files are read as one block and split once, rather than line by
        line through the pipeline - noticeably faster on big text files.
      - -CacheFile enables incremental re-scans: a file whose size and modified
        time haven't changed since the last run (with the same filters/settings)
        is skipped entirely and its prior result is reused.

      ROBUSTNESS
      - Any file (of any type) that hits an unexpected error is now skipped with
        a warning instead of stopping the whole run (already true from before;
        reconfirmed here).
      - Reads automatically retry with backoff if a file is transiently locked by
        another program, and give up with a clear message after -MaxRetries.
      - A per-file -FileTimeoutSeconds guards against a stalled network share
        blocking the whole run.
      - The folder walk tracks resolved real paths to avoid getting stuck in a
        symlink/junction loop.
      - Text files without a byte-order-mark are sniffed for valid UTF-8 and
        fall back to Windows-1252 instead of producing garbled characters.
      - -DryRun lists what would be searched (counts, breakdown by extension)
        without reading any file content, so you can sanity-check filters/
        excludes/extensions before committing to a long run.

      UX / REPORT
      - The console progress bar now shows a live running hit count.
      - The HTML report has a jump-to table of contents for large result sets.
      - The report respects your OS/browser dark mode automatically (pure CSS,
        no toggle needed).
      - A small CSS-only bar chart visualizes hits per filter in the summary.
      - PDF entries whose extracted text looks unreliable (a common symptom of
        embedded/subsetted fonts) are flagged so you know which ones to check
        manually rather than trusting a possible false negative.

.PARAMETER SearchPath
    Root folder to search recursively. Must already exist.

.PARAMETER Filter
    One or more keywords/phrases to search for, e.g. -Filter "invoice","overdue".

.PARAMETER OutputFolder
    Folder where the report(s) will be written. Created automatically if it does
    not already exist. Must be a location you have write access to.

.PARAMETER OutputName
    Optional base file name for the report(s). Defaults to a timestamped name.

.PARAMETER MatchMode
    'AnyLine' (default), 'AllInFile', or 'Proximity'.

.PARAMETER ProximityLines
    Only used when -MatchMode Proximity. Maximum line span allowed between the
    filters for a file to qualify. Default 5.

.PARAMETER ExcludeFilter
    One or more terms that suppress a hit if found.

.PARAMETER ExcludeScope
    'Line' (default) suppresses just the offending line. 'File' drops the whole
    file if any exclude term appears anywhere in it.

.PARAMETER WholeWord
    Match -Filter/-ExcludeFilter values as whole words only (literal mode only).

.PARAMETER GroupBy
    'Created' (default), 'Modified', or 'None'.

.PARAMETER Extensions
    File extensions to include (default = common text/document/code types, plus
    .docx/.pptx/.rtf/.pdf). Pass -Extensions '*' to attempt every file regardless
    of extension.

.PARAMETER ExcludeFolder
    Optional list of folder-name fragments to skip entirely.

.PARAMETER UseRegex
    Treat each -Filter/-ExcludeFilter value as a regular expression.

.PARAMETER MaxFileSizeMB
    Skip any file larger than this size, in megabytes. Default 50.

.PARAMETER MaxEmbedLines
    Maximum number of extracted lines to embed inline per file in the report's
    expandable view. Default 4000.

.PARAMETER PdfTimeoutSeconds
    Overall time budget for extracting text from any single PDF. Default 15.

.PARAMETER IncludeHidden
    Also search hidden and system files/folders. Off by default.

.PARAMETER OpenReport
    Automatically open the finished HTML report in your default browser when done.

.PARAMETER ExportCsv
    Also write a .csv file (one row per hit) alongside the HTML report.

.PARAMETER ExportJson
    Also write a .json file (one row per hit) alongside the HTML report.

.PARAMETER Parallel
    Process files concurrently. Requires PowerShell 7+; ignored with a note on
    Windows PowerShell 5.1.

.PARAMETER ThrottleLimit
    Maximum concurrent files when -Parallel is used. Default 5.

.PARAMETER CacheFile
    Path to a small JSON cache file. When set, a file whose size/modified time
    match the cache from a prior run with the SAME filters/settings is reused
    instead of re-read. The cache is created if it doesn't exist yet, and is
    automatically pruned of files that no longer exist.

.PARAMETER DryRun
    List what would be searched (file counts by extension) without reading any
    file content or writing a report.

.PARAMETER MaxRetries
    Retry attempts for a file that's transiently locked by another program.
    Default 3.

.PARAMETER RetryDelayMs
    Base delay between retries in milliseconds (increases each attempt).
    Default 250.

.PARAMETER FileTimeoutSeconds
    Per-file hard timeout for the initial read, guarding against a stalled
    network location. Default 30.

.EXAMPLE
    .\Search-TextInFiles.ps1 -SearchPath "D:\Projects" -Filter "TODO","FIXME" `
        -OutputFolder "D:\Reports" -Parallel -CacheFile "D:\Reports\.cache.json"

.EXAMPLE
    .\Search-TextInFiles.ps1 -SearchPath "D:\Huge Archive" -Filter "test" `
        -OutputFolder "D:\Reports" -DryRun

.NOTES
    Tested against Windows PowerShell 5.1 and PowerShell 7+. Pure built-in cmdlets
    and .NET types only - no external modules, no admin rights, no internet
    access anywhere.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$SearchPath,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string[]]$Filter,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$OutputFolder,

    [string]$OutputName,

    [ValidateSet('AnyLine', 'AllInFile', 'Proximity')]
    [string]$MatchMode = 'AnyLine',

    [ValidateRange(0, 1000000)]
    [int]$ProximityLines = 5,

    [string[]]$ExcludeFilter = @(),

    [ValidateSet('Line', 'File')]
    [string]$ExcludeScope = 'Line',

    [switch]$WholeWord,

    [ValidateSet('Created', 'Modified', 'None')]
    [string]$GroupBy = 'Created',

    [string[]]$Extensions = @(
        '.txt', '.log', '.csv', '.tsv', '.md', '.ini', '.cfg', '.conf',
        '.xml', '.json', '.yaml', '.yml', '.htm', '.html',
        '.ps1', '.psm1', '.bat', '.cmd', '.py', '.js', '.ts', '.cs',
        '.java', '.sql', '.rtf', '.docx', '.pptx', '.pdf'
    ),

    [string[]]$ExcludeFolder = @(),

    [switch]$UseRegex,

    [double]$MaxFileSizeMB = 50,

    [int]$MaxEmbedLines = 4000,

    [int]$PdfTimeoutSeconds = 15,

    [switch]$IncludeHidden,

    [switch]$OpenReport,

    [switch]$ExportCsv,

    [switch]$ExportJson,

    [switch]$Parallel,

    [ValidateRange(1, 64)]
    [int]$ThrottleLimit = 5,

    [string]$CacheFile,

    [switch]$DryRun,

    [ValidateRange(0, 10)]
    [int]$MaxRetries = 3,

    [ValidateRange(10, 60000)]
    [int]$RetryDelayMs = 250,

    [ValidateRange(1, 3600)]
    [int]$FileTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ----------------------------------------------------------------------------
# Helper functions
# ----------------------------------------------------------------------------

function ConvertTo-HtmlSafe {
    <# Minimal, dependency-free HTML entity encoding. #>
    param([AllowNull()][string]$Text)
    if ([string]::IsNullOrEmpty($Text)) { return '' }
    $Text.Replace('&', '&amp;').Replace('<', '&lt;').Replace('>', '&gt;').Replace('"', '&quot;')
}

function Get-FileUri {
    <# Build a file:/// URI from a local absolute path for use as a hyperlink. #>
    param([string]$Path)
    try {
        return ([System.Uri]$Path).AbsoluteUri
    }
    catch {
        return $null
    }
}

function Remove-SurroundingQuotes {
    <# Strips one matching pair of leading/trailing straight quotes, if present. #>
    param([string]$Text)
    if ([string]::IsNullOrEmpty($Text)) { return $Text }
    $t = $Text.Trim()
    if ($t.Length -ge 2) {
        if (($t.StartsWith('"') -and $t.EndsWith('"')) -or ($t.StartsWith("'") -and $t.EndsWith("'"))) {
            $t = $t.Substring(1, $t.Length - 2)
        }
    }
    return $t
}

function Get-TextFromBytes {
    <#
        Converts bytes to text with basic encoding detection: BOM first, then a
        strict UTF-8 validity check, falling back to Windows-1252 (the common
        legacy default) for files with neither - avoids garbled characters on
        older non-UTF8 text files instead of silently mis-decoding them.
    #>
    param([byte[]]$Bytes)

    if (-not $Bytes -or $Bytes.Length -eq 0) { return '' }

    if ($Bytes.Length -ge 3 -and $Bytes[0] -eq 0xEF -and $Bytes[1] -eq 0xBB -and $Bytes[2] -eq 0xBF) {
        return [System.Text.Encoding]::UTF8.GetString($Bytes, 3, $Bytes.Length - 3)
    }
    if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 0xFF -and $Bytes[1] -eq 0xFE) {
        return [System.Text.Encoding]::Unicode.GetString($Bytes, 2, $Bytes.Length - 2)
    }
    if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 0xFE -and $Bytes[1] -eq 0xFF) {
        return [System.Text.Encoding]::BigEndianUnicode.GetString($Bytes, 2, $Bytes.Length - 2)
    }

    try {
        $utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
        return $utf8Strict.GetString($Bytes)
    }
    catch {
        try {
            return [System.Text.Encoding]::GetEncoding(1252).GetString($Bytes)
        }
        catch {
            return [System.Text.Encoding]::GetEncoding('ISO-8859-1').GetString($Bytes)
        }
    }
}

function Read-FileBytesRobust {
    <#
        Reads a whole file as bytes with retry-with-backoff for transient
        sharing-violation errors (a file someone else has open) and a hard
        timeout via async FileStream reads, so a stalled network share can't
        block the whole run. Static .NET method calls surface as
        MethodInvocationException in PowerShell with the real error nested in
        InnerException, so that's what gets inspected to decide whether a
        failure is worth retrying.
    #>
    param(
        [string]$Path,
        [int]$TimeoutSeconds = 30,
        [int]$MaxRetries = 3,
        [int]$RetryDelayMs = 250
    )

    $attempt = 0
    while ($true) {
        $attempt++
        try {
            $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
            try {
                $length = $fs.Length
                if ($length -eq 0) { return , (New-Object byte[] 0) }
                $buffer = New-Object byte[] $length
                $ar = $fs.BeginRead($buffer, 0, [int]$length, $null, $null)
                if ($ar.AsyncWaitHandle.WaitOne([TimeSpan]::FromSeconds($TimeoutSeconds))) {
                    [void]$fs.EndRead($ar)
                    return , $buffer
                }
                else {
                    throw [System.TimeoutException]::new("Timed out reading '$Path' after $TimeoutSeconds second(s).")
                }
            }
            finally {
                $fs.Dispose()
            }
        }
        catch [System.TimeoutException] {
            throw
        }
        catch {
            $inner = if ($_.Exception.InnerException) { $_.Exception.InnerException } else { $_.Exception }
            $isShareViolation = ($inner -is [System.IO.IOException]) -and ($inner -isnot [System.IO.FileNotFoundException]) -and ($inner -isnot [System.IO.DirectoryNotFoundException])
            if ($isShareViolation -and $attempt -le $MaxRetries) {
                Start-Sleep -Milliseconds ($RetryDelayMs * $attempt)
                continue
            }
            throw
        }
    }
}

function Test-IsProbablyBinaryBytes {
    <# Cheap heuristic: NUL bytes in the first chunk essentially never appear in real text files. #>
    param([byte[]]$Bytes)
    if (-not $Bytes -or $Bytes.Length -eq 0) { return $false }
    $checkLen = [Math]::Min($Bytes.Length, 4096)
    for ($i = 0; $i -lt $checkLen; $i++) {
        if ($Bytes[$i] -eq 0) { return $true }
    }
    return $false
}

function Get-DocxPlainText {
    <#
        Best-effort, dependency-free extraction of visible text from a .docx
        file's already-read bytes (a zip archive) - reads word/document.xml
        directly out of it and strips the XML tags. Returns $null on failure so
        the caller skips the file gracefully.
    #>
    param([byte[]]$Bytes)
    try {
        Add-Type -AssemblyName System.IO.Compression -ErrorAction SilentlyContinue
        $ms = New-Object System.IO.MemoryStream(, $Bytes)
        $zip = New-Object System.IO.Compression.ZipArchive($ms, [System.IO.Compression.ZipArchiveMode]::Read)
        try {
            $entry = $zip.Entries | Where-Object { $_.FullName -eq 'word/document.xml' }
            if (-not $entry) { return $null }
            $reader = New-Object System.IO.StreamReader($entry.Open())
            try {
                $xmlText = $reader.ReadToEnd()
            }
            finally {
                $reader.Dispose()
            }
        }
        finally {
            $zip.Dispose()
        }
        $xmlText = $xmlText -replace '</w:p>', "`n"
        $xmlText = $xmlText -replace '<w:br\s*/?>', "`n"
        $xmlText = $xmlText -replace '<[^>]+>', ''
        $xmlText = $xmlText.Replace('&amp;', '&').Replace('&lt;', '<').Replace('&gt;', '>').Replace('&quot;', '"').Replace('&apos;', "'")
        $docxLines = $xmlText -split "`r?`n"
        return , $docxLines
    }
    catch {
        return $null
    }
}

function Get-PptxPlainText {
    <#
        Best-effort, dependency-free extraction of visible text from a .pptx
        file's already-read bytes. Reads ppt/slides/slideN.xml entries in slide
        order and strips XML tags, inserting a "--- Slide N ---" marker between
        slides for orientation. Returns $null on failure.
    #>
    param([byte[]]$Bytes)
    try {
        Add-Type -AssemblyName System.IO.Compression -ErrorAction SilentlyContinue
        $ms = New-Object System.IO.MemoryStream(, $Bytes)
        $zip = New-Object System.IO.Compression.ZipArchive($ms, [System.IO.Compression.ZipArchiveMode]::Read)
        try {
            $slideEntries = $zip.Entries |
                Where-Object { $_.FullName -match '^ppt/slides/slide(\d+)\.xml$' } |
                Sort-Object { [int]([regex]::Match($_.FullName, '\d+').Value) }

            if (-not $slideEntries -or @($slideEntries).Count -eq 0) { return $null }

            $allLines = New-Object System.Collections.Generic.List[string]
            $slideNum = 0
            foreach ($entry in $slideEntries) {
                $slideNum++
                $reader = New-Object System.IO.StreamReader($entry.Open())
                try {
                    $xmlText = $reader.ReadToEnd()
                }
                finally {
                    $reader.Dispose()
                }
                $xmlText = $xmlText -replace '</a:p>', "`n"
                $xmlText = $xmlText -replace '<a:br\s*/?>', "`n"
                $xmlText = $xmlText -replace '<[^>]+>', ''
                $xmlText = $xmlText.Replace('&amp;', '&').Replace('&lt;', '<').Replace('&gt;', '>').Replace('&quot;', '"').Replace('&apos;', "'")

                [void]$allLines.Add("--- Slide $slideNum ---")
                foreach ($l in ($xmlText -split "`r?`n")) {
                    if ($l.Trim().Length -gt 0) { [void]$allLines.Add($l) }
                }
            }
            return , $allLines.ToArray()
        }
        finally {
            $zip.Dispose()
        }
    }
    catch {
        return $null
    }
}

function Get-RtfPlainText {
    <#
        Small dependency-free RTF-to-text converter operating on already-read
        bytes. Walks the RTF character by character tracking group nesting,
        skips destination groups with no visible document text, and converts
        \par/\line/\tab and \uNNNN / \'hh escapes into real characters. Not a
        full RTF spec implementation. Returns $null if it doesn't look like RTF.
    #>
    param([byte[]]$Bytes)

    $raw = Get-TextFromBytes -Bytes $Bytes
    if (-not $raw.StartsWith('{\rtf')) { return $null }

    $ignoreGroups = @(
        'fonttbl', 'colortbl', 'stylesheet', 'info', 'generator', 'pict',
        'object', 'footer', 'footerf', 'footerl', 'footerr',
        'header', 'headerf', 'headerl', 'headerr',
        'footnote', 'xe', 'tc', 'field', 'shppict', 'nonshppict',
        'themedata', 'colorschememapping', 'datastore', 'listtable', 'listoverridetable'
    )

    $sb = New-Object System.Text.StringBuilder
    $len = $raw.Length
    $i = 0
    $depth = 0
    $skipDepth = -1

    while ($i -lt $len) {
        $ch = $raw[$i]

        if ($ch -eq '{') { $depth++; $i++; continue }
        if ($ch -eq '}') {
            if ($skipDepth -ge 0 -and $depth -le $skipDepth) { $skipDepth = -1 }
            $depth--
            $i++
            continue
        }

        if ($ch -eq '\') {
            $i++
            if ($i -ge $len) { break }
            $c2 = $raw[$i]

            if ($c2 -eq '*') {
                $i++
                if ($skipDepth -lt 0) { $skipDepth = $depth }
                continue
            }
            elseif ($c2 -match '[a-zA-Z]') {
                $wordStart = $i
                while ($i -lt $len -and $raw[$i] -match '[a-zA-Z]') { $i++ }
                $word = $raw.Substring($wordStart, $i - $wordStart)

                $numStart = $i
                if ($i -lt $len -and $raw[$i] -eq '-') { $i++ }
                while ($i -lt $len -and [char]::IsDigit($raw[$i])) { $i++ }
                $numStr = $raw.Substring($numStart, $i - $numStart)

                if ($i -lt $len -and $raw[$i] -eq ' ') { $i++ }

                if ($ignoreGroups -contains $word) {
                    if ($skipDepth -lt 0) { $skipDepth = $depth }
                }
                elseif ($word -in @('par', 'line', 'row', 'cell')) {
                    if ($skipDepth -lt 0) { [void]$sb.Append("`n") }
                }
                elseif ($word -eq 'tab') {
                    if ($skipDepth -lt 0) { [void]$sb.Append("`t") }
                }
                elseif ($word -eq 'u') {
                    if ($skipDepth -lt 0 -and $numStr -ne '') {
                        $codepoint = [int]$numStr
                        if ($codepoint -lt 0) { $codepoint += 65536 }
                        try { [void]$sb.Append([char]$codepoint) } catch { }
                    }
                    if ($i -lt $len -and $raw[$i] -notin @('\', '{', '}')) { $i++ }
                }
                continue
            }
            else {
                if ($c2 -eq "'") {
                    $i++
                    if ($i + 1 -lt $len) {
                        $hex = $raw.Substring($i, 2)
                        $i += 2
                        if ($skipDepth -lt 0) {
                            try {
                                $byteVal = [Convert]::ToInt32($hex, 16)
                                [void]$sb.Append([char]$byteVal)
                            }
                            catch { }
                        }
                    }
                    continue
                }
                elseif ($c2 -in @('\', '{', '}')) {
                    if ($skipDepth -lt 0) { [void]$sb.Append($c2) }
                    $i++
                    continue
                }
                elseif ($c2 -eq '~') {
                    if ($skipDepth -lt 0) { [void]$sb.Append(' ') }
                    $i++
                    continue
                }
                else {
                    $i++
                    continue
                }
            }
        }

        if ($skipDepth -lt 0) { [void]$sb.Append($ch) }
        $i++
    }

    $rtfLines = $sb.ToString() -split "`r?`n"
    return , $rtfLines
}

function ConvertFrom-Ascii85 {
    <#
        Decodes PDF-style ASCII85 (Adobe variant: lowercase 'z' shorthand for
        four zero bytes - IMPORTANT: this must be a case-SENSITIVE check, since
        PowerShell's default -eq is case-insensitive and would wrongly treat a
        capital 'Z' data character as the shorthand too. Optional '~>' end
        marker, whitespace ignored.
    #>
    param([string]$Text)

    $t = $Text -replace '\s', ''
    if ($t.EndsWith('~>')) { $t = $t.Substring(0, $t.Length - 2) }

    $outBytes = New-Object System.Collections.Generic.List[byte]
    $group = New-Object 'int[]' 5
    $count = 0

    for ($i = 0; $i -lt $t.Length; $i++) {
        $ch = $t[$i]
        if ($ch -ceq 'z' -and $count -eq 0) {
            [void]$outBytes.Add(0); [void]$outBytes.Add(0); [void]$outBytes.Add(0); [void]$outBytes.Add(0)
            continue
        }
        $val = [int][char]$ch - 33
        if ($val -lt 0 -or $val -gt 84) { continue }
        $group[$count] = $val
        $count++
        if ($count -eq 5) {
            $num = [uint64]0
            foreach ($g in $group) { $num = $num * 85 + [uint64]$g }
            [void]$outBytes.Add([byte](($num -shr 24) -band 0xFF))
            [void]$outBytes.Add([byte](($num -shr 16) -band 0xFF))
            [void]$outBytes.Add([byte](($num -shr 8) -band 0xFF))
            [void]$outBytes.Add([byte]($num -band 0xFF))
            $count = 0
        }
    }

    if ($count -gt 0) {
        $padCount = 5 - $count
        for ($p = 0; $p -lt $padCount; $p++) { $group[$count + $p] = 84 }
        $num = [uint64]0
        foreach ($g in $group) { $num = $num * 85 + [uint64]$g }
        $tmp = @(
            [byte](($num -shr 24) -band 0xFF),
            [byte](($num -shr 16) -band 0xFF),
            [byte](($num -shr 8) -band 0xFF),
            [byte]($num -band 0xFF)
        )
        for ($k = 0; $k -lt $count - 1; $k++) { [void]$outBytes.Add($tmp[$k]) }
    }

    return , $outBytes.ToArray()
}

function Get-PdfPlainText {
    <#
        Lightweight, dependency-free, BEST-EFFORT PDF text extractor operating
        on already-read bytes. Finds stream...endstream blocks, decodes
        /ASCII85Decode and/or /FlateDecode filtered streams, and pulls text out
        of Tj/TJ show-text operators.

        PERFORMANCE / HANG SAFEGUARDS - skips streams whose own dictionary marks
        them as images, embedded font programs, ICC profiles, or metadata
        without decompressing them; applies a short timeout to every regex
        match attempt; caps how much of any one stream gets scanned; stops
        after $OverallTimeoutSeconds total and keeps whatever text was already
        found.

        LIMITATIONS: no OCR; does not resolve ToUnicode CMaps, so PDFs with
        embedded/subsetted fonts (common from LaTeX/pdflatex) may extract as
        garbled or missing text; filters other than ASCII85Decode/FlateDecode
        are not handled. Returns $null if nothing could be extracted.
    #>
    param(
        [byte[]]$Bytes,
        [int]$OverallTimeoutSeconds = 15
    )

    $latin1 = [System.Text.Encoding]::GetEncoding('ISO-8859-1')
    $raw = $latin1.GetString($Bytes)

    $lines = New-Object System.Collections.Generic.List[string]
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $regexTimeout = [TimeSpan]::FromSeconds(2)
    $truncatedByTime = $false
    $maxContentChars = 2000000

    $streamRegex = New-Object System.Text.RegularExpressions.Regex(
        '(?s)(.{0,400}?)stream\r?\n(.*?)endstream',
        [System.Text.RegularExpressions.RegexOptions]::None,
        $regexTimeout
    )
    $textRegex = New-Object System.Text.RegularExpressions.Regex(
        '\((?:\\.|[^()])*\)',
        [System.Text.RegularExpressions.RegexOptions]::None,
        $regexTimeout
    )
    $skipMarkers = '/Image|/FontFile|/ICCBased|/Metadata'

    $match = $null
    try {
        $match = $streamRegex.Match($raw)
    }
    catch {
        return $null
    }

    while ($match -and $match.Success) {
        if ($sw.Elapsed.TotalSeconds -ge $OverallTimeoutSeconds) {
            $truncatedByTime = $true
            break
        }

        try {
            $header = $match.Groups[1].Value

            if ($header -notmatch $skipMarkers) {
                $streamText = $match.Groups[2].Value

                if ($streamText.Length -gt 0) {
                    $hasAscii85 = $header -match '/ASCII85Decode'
                    $hasFlate = $header -match '/FlateDecode'
                    $contentBytes = $null

                    $workingBytes = $null
                    if ($hasAscii85) {
                        try { $workingBytes = ConvertFrom-Ascii85 -Text $streamText } catch { $workingBytes = $null }
                    }
                    else {
                        $workingBytes = $latin1.GetBytes($streamText)
                    }

                    if ($workingBytes -and $workingBytes.Length -gt 0) {
                        if ($hasFlate) {
                            if ($workingBytes.Length -gt 2) {
                                try {
                                    $ms = New-Object System.IO.MemoryStream(, $workingBytes[2..($workingBytes.Length - 1)])
                                    $ds = New-Object System.IO.Compression.DeflateStream($ms, [System.IO.Compression.CompressionMode]::Decompress)
                                    $outMs = New-Object System.IO.MemoryStream
                                    $ds.CopyTo($outMs)
                                    $ds.Dispose()
                                    $contentBytes = $outMs.ToArray()
                                }
                                catch {
                                    $contentBytes = $null
                                }
                            }
                        }
                        else {
                            $contentBytes = $workingBytes
                        }
                    }

                    if ($contentBytes -and $contentBytes.Length -gt 0) {
                        $contentLen = [Math]::Min($contentBytes.Length, $maxContentChars)
                        $content = $latin1.GetString($contentBytes, 0, $contentLen)

                        $looksLikeText = $false
                        try {
                            $looksLikeText = [System.Text.RegularExpressions.Regex]::IsMatch($content, '\bTj\b|\bTJ\b', [System.Text.RegularExpressions.RegexOptions]::None, $regexTimeout)
                        }
                        catch { }

                        if ($looksLikeText) {
                            try {
                                foreach ($tm in $textRegex.Matches($content)) {
                                    $inner = $tm.Value.Substring(1, $tm.Value.Length - 2)

                                    $inner = [regex]::Replace($inner, '\\([\\()nrtbf]|[0-7]{1,3})', {
                                            param($em)
                                            $g = $em.Groups[1].Value
                                            switch -Regex ($g) {
                                                '^[0-7]{1,3}$' { try { [string][char]([Convert]::ToInt32($g, 8)) } catch { '' } }
                                                '^n$' { "`n" }
                                                '^r$' { "`r" }
                                                '^t$' { "`t" }
                                                '^b$' { [string][char]8 }
                                                '^f$' { [string][char]12 }
                                                '^\($' { '(' }
                                                '^\)$' { ')' }
                                                '^\\$' { '\' }
                                                default { $g }
                                            }
                                        })

                                    if ($inner.Trim().Length -gt 0) {
                                        [void]$lines.Add($inner)
                                    }
                                }
                            }
                            catch { }
                        }
                    }
                }
            }
        }
        catch {
            # Any per-stream failure (including a regex timeout) just means we
            # move on to the next stream rather than losing the whole file.
        }

        try {
            $match = $match.NextMatch()
        }
        catch {
            $truncatedByTime = $true
            break
        }
    }

    if ($truncatedByTime -and $lines.Count -gt 0) {
        [void]$lines.Add("[... PDF text extraction stopped early after $OverallTimeoutSeconds seconds on this large/complex file - some text may be missing ...]")
    }

    if ($lines.Count -eq 0) { return $null }
    return , $lines.ToArray()
}

function Test-PdfExtractionLooksReliable {
    <#
        Cheap heuristic to flag PDFs whose extracted text is probably garbled -
        typically PDFs with embedded/subsetted fonts using custom glyph
        encodings (common from LaTeX/pdflatex) that this extractor can't decode
        correctly. Not a guarantee either way - just a hint to double-check that
        particular file manually rather than trusting a possible false negative.
    #>
    param([string[]]$Lines)

    if (-not $Lines -or $Lines.Count -eq 0) { return $true }

    $sample = @($Lines | Select-Object -First 200)
    $sampleText = $sample -join ' '
    if ($sampleText.Length -eq 0) { return $true }

    $letters = 0
    $spaces = 0
    $printable = 0
    $total = $sampleText.Length

    foreach ($ch in $sampleText.ToCharArray()) {
        if ([char]::IsLetter($ch)) { $letters++ }
        if ($ch -eq ' ') { $spaces++ }
        if (-not [char]::IsControl($ch)) { $printable++ }
    }

    $letterRatio = $letters / $total
    $spaceRatio = $spaces / $total
    $printableRatio = $printable / $total

    return ($letterRatio -gt 0.35 -and $spaceRatio -gt 0.08 -and $printableRatio -gt 0.9)
}

function Format-MatchLine {
    <#
        HTML-encodes a line and wraps every actual matched span - literal,
        whole-word, or regex filters alike - in <mark>. Finds real character
        ranges in the RAW line first (so regex mode is highlighted too, not
        just literal mode), merges overlapping ranges from different filters,
        then encodes each plain/highlighted piece in turn.
    #>
    param(
        [string]$Line,
        [string[]]$MatchedFilters,
        [bool]$RegexMode,
        [bool]$WholeWordMode
    )

    if ([string]::IsNullOrEmpty($Line)) { return ConvertTo-HtmlSafe $Line }

    $ranges = New-Object System.Collections.Generic.List[int[]]

    foreach ($f in $MatchedFilters) {
        if ([string]::IsNullOrEmpty($f)) { continue }

        $pattern = if ($RegexMode) { $f } elseif ($WholeWordMode) { '\b' + [regex]::Escape($f) + '\b' } else { [regex]::Escape($f) }

        try {
            foreach ($m in [regex]::Matches($Line, $pattern, 'IgnoreCase')) {
                if ($m.Length -gt 0) { $ranges.Add(@($m.Index, ($m.Index + $m.Length))) }
            }
        }
        catch {
            # An invalid/expensive user regex here just means this filter isn't
            # highlighted - the line itself is still shown safely below.
        }
    }

    if ($ranges.Count -eq 0) { return ConvertTo-HtmlSafe $Line }

    $sorted = @($ranges | Sort-Object { $_[0] })
    $merged = New-Object System.Collections.Generic.List[int[]]
    foreach ($r in $sorted) {
        if ($merged.Count -gt 0 -and $r[0] -le $merged[$merged.Count - 1][1]) {
            if ($r[1] -gt $merged[$merged.Count - 1][1]) { $merged[$merged.Count - 1][1] = $r[1] }
        }
        else {
            $merged.Add(@($r[0], $r[1]))
        }
    }

    $sb = New-Object System.Text.StringBuilder
    $pos = 0
    foreach ($m in $merged) {
        if ($m[0] -gt $pos) {
            [void]$sb.Append((ConvertTo-HtmlSafe $Line.Substring($pos, $m[0] - $pos)))
        }
        [void]$sb.Append('<mark>')
        [void]$sb.Append((ConvertTo-HtmlSafe $Line.Substring($m[0], $m[1] - $m[0])))
        [void]$sb.Append('</mark>')
        $pos = $m[1]
    }
    if ($pos -lt $Line.Length) {
        [void]$sb.Append((ConvertTo-HtmlSafe $Line.Substring($pos)))
    }

    return $sb.ToString()
}

function Get-MinLineRangeAcrossFilters {
    <#
        Given a hashtable mapping each filter to its sorted, distinct array of
        hit-line-numbers within one file, returns the smallest line span (max -
        min) of any combination that includes at least one line per filter -
        the classic "smallest range covering one element from each list"
        problem, solved by advancing whichever list currently sits at the
        minimum value. Assumes every filter in $Filters has at least one entry.
    #>
    param(
        [hashtable]$FilterLineLists,
        [string[]]$Filters
    )

    $lists = New-Object 'System.Collections.Generic.List[object]'
    foreach ($f in $Filters) {
        [void]$lists.Add(@($FilterLineLists[$f]))
    }
    $k = $lists.Count
    if ($k -eq 0) { return [int]::MaxValue }

    $ptr = New-Object 'int[]' $k
    $bestRange = [int]::MaxValue

    while ($true) {
        $vals = New-Object 'int[]' $k
        $exhausted = $false
        for ($i = 0; $i -lt $k; $i++) {
            if ($ptr[$i] -ge $lists[$i].Length) { $exhausted = $true; break }
            $vals[$i] = $lists[$i][$ptr[$i]]
        }
        if ($exhausted) { break }

        $minVal = $vals[0]; $maxVal = $vals[0]; $minIdx = 0
        for ($i = 1; $i -lt $k; $i++) {
            if ($vals[$i] -lt $minVal) { $minVal = $vals[$i]; $minIdx = $i }
            if ($vals[$i] -gt $maxVal) { $maxVal = $vals[$i] }
        }

        $range = $maxVal - $minVal
        if ($range -lt $bestRange) { $bestRange = $range }

        $ptr[$minIdx]++
    }

    return $bestRange
}

function Build-CombinedRegex {
    <#
        Builds one alternation pattern from all filters, for a cheap single
        pre-check per line before running the slower per-filter loop that
        determines which specific filter(s) matched. Returns $null if it can't
        build a valid combined pattern (e.g. a broken user regex) - the caller
        then just checks every filter on every line as before, which is always
        correct, only slower.
    #>
    param(
        [string[]]$Filters,
        [bool]$UseRegex,
        [bool]$WholeWord
    )
    if (-not $Filters -or $Filters.Count -eq 0) { return $null }

    try {
        $parts = foreach ($f in $Filters) {
            if ($UseRegex) { "(?:$f)" }
            elseif ($WholeWord) { '\b' + [regex]::Escape($f) + '\b' }
            else { [regex]::Escape($f) }
        }
        $pattern = '(?:' + ($parts -join '|') + ')'
        return New-Object System.Text.RegularExpressions.Regex($pattern, 'IgnoreCase, Compiled')
    }
    catch {
        return $null
    }
}

function Get-FilesSafely {
    <#
        Manual recursive directory walk (instead of Get-ChildItem -Recurse) that
        tracks visited real (resolved) directory paths to guard against a
        symlink/junction cycle, which -Recurse alone doesn't protect against.
        Inaccessible folders are counted and skipped, never fatal.
    #>
    param(
        [string]$RootPath,
        [switch]$IncludeHidden,
        [ref]$EnumErrorCount
    )

    $visited = New-Object 'System.Collections.Generic.HashSet[string]'([StringComparer]::OrdinalIgnoreCase)
    $results = New-Object System.Collections.Generic.List[System.IO.FileInfo]
    $stack = New-Object System.Collections.Generic.Stack[string]
    $stack.Push($RootPath)

    while ($stack.Count -gt 0) {
        $dir = $stack.Pop()

        try {
            $resolvedDir = (Get-Item -LiteralPath $dir -Force -ErrorAction Stop).FullName
        }
        catch {
            $EnumErrorCount.Value = $EnumErrorCount.Value + 1
            continue
        }

        if (-not $visited.Add($resolvedDir)) {
            continue
        }

        try {
            $childDirs = [System.IO.Directory]::GetDirectories($dir)
            $childFiles = [System.IO.Directory]::GetFiles($dir)
        }
        catch {
            $EnumErrorCount.Value = $EnumErrorCount.Value + 1
            continue
        }

        foreach ($f in $childFiles) {
            try {
                $fi = New-Object System.IO.FileInfo($f)
                if (-not $IncludeHidden -and ($fi.Attributes -band [System.IO.FileAttributes]::Hidden)) { continue }
                $results.Add($fi)
            }
            catch {
                $EnumErrorCount.Value = $EnumErrorCount.Value + 1
            }
        }

        foreach ($d in $childDirs) {
            try {
                $di = New-Object System.IO.DirectoryInfo($d)
                if (-not $IncludeHidden -and ($di.Attributes -band [System.IO.FileAttributes]::Hidden)) { continue }
                $stack.Push($d)
            }
            catch {
                $EnumErrorCount.Value = $EnumErrorCount.Value + 1
            }
        }
    }

    return , $results
}

function Invoke-SingleFileSearch {
    <#
        Processes exactly one file end to end: robust byte read, format-aware
        text extraction, exclude/include line matching (using precompiled
        regex caches and a fast combined pre-check where possible), and
        AllInFile/Proximity gating. Returns one uniform result object
        regardless of outcome, and never throws - any unexpected failure comes
        back as Status='UnexpectedError' with the message attached, so a
        single bad file can never take down a whole run (sequential or
        parallel).
    #>
    param(
        [Parameter(Mandatory = $true)]$File,
        [string[]]$Filter,
        [string[]]$ExcludeFilter,
        [string]$MatchMode,
        [string]$ExcludeScope,
        [int]$ProximityLines,
        [bool]$UseRegex,
        [bool]$WholeWord,
        [int]$MaxEmbedLines,
        [int]$PdfTimeoutSeconds,
        [int]$FileTimeoutSeconds,
        [int]$MaxRetries,
        [int]$RetryDelayMs,
        $CombinedFilterRegex,
        $CombinedExcludeRegex,
        [hashtable]$WholeWordFilterRegex,
        [hashtable]$WholeWordExcludeRegex,
        [hashtable]$CompiledFilterRegex,
        [hashtable]$CompiledExcludeRegex
    )

    $result = [PSCustomObject]@{
        FullName          = $File.FullName
        Status            = 'NoHit'
        Hits              = @()
        Created           = $File.CreationTime
        Modified          = $File.LastWriteTime
        LinesCache        = @()
        TotalLineCount    = 0
        ProximityMinRange = $null
        LowConfidencePdf  = $false
        ErrorMessage      = $null
    }

    try {
        $ext = $File.Extension.ToLowerInvariant()

        try {
            $bytes = Read-FileBytesRobust -Path $File.FullName -TimeoutSeconds $FileTimeoutSeconds -MaxRetries $MaxRetries -RetryDelayMs $RetryDelayMs
        }
        catch {
            $result.Status = 'ReadError'
            $result.ErrorMessage = $_.Exception.Message
            return $result
        }

        $lines = $null
        $lowConfidence = $false

        if ($ext -eq '.docx') {
            $lines = Get-DocxPlainText -Bytes $bytes
        }
        elseif ($ext -eq '.pptx') {
            $lines = Get-PptxPlainText -Bytes $bytes
        }
        elseif ($ext -eq '.pdf') {
            $lines = Get-PdfPlainText -Bytes $bytes -OverallTimeoutSeconds $PdfTimeoutSeconds
            if ($lines) { $lowConfidence = -not (Test-PdfExtractionLooksReliable -Lines $lines) }
        }
        elseif ($ext -eq '.rtf') {
            $lines = Get-RtfPlainText -Bytes $bytes
        }
        else {
            if (Test-IsProbablyBinaryBytes -Bytes $bytes) {
                $result.Status = 'Binary'
                return $result
            }
            $text = Get-TextFromBytes -Bytes $bytes
            $lines = $text -split "`r?`n"
        }

        if (-not $lines -or $lines.Count -eq 0) {
            $result.Status = 'ReadError'
            return $result
        }

        $result.LowConfidencePdf = $lowConfidence

        $fileResults = New-Object System.Collections.Generic.List[object]
        $fileMatchedFilterSet = New-Object 'System.Collections.Generic.HashSet[string]'([StringComparer]::OrdinalIgnoreCase)
        $fileHasExcludeMatch = $false

        for ($i = 0; $i -lt $lines.Count; $i++) {
            $line = $lines[$i]
            if ($null -eq $line) { continue }

            if ($ExcludeFilter.Count -gt 0) {
                $excludeCandidate = $true
                if ($CombinedExcludeRegex) { $excludeCandidate = $CombinedExcludeRegex.IsMatch($line) }
                if ($excludeCandidate) {
                    $isExcludedLine = $false
                    foreach ($xf in $ExcludeFilter) {
                        if ($UseRegex) {
                            try {
                                if ($CompiledExcludeRegex -and $CompiledExcludeRegex.ContainsKey($xf) -and $CompiledExcludeRegex[$xf].IsMatch($line)) { $isExcludedLine = $true; break }
                            }
                            catch { }
                        }
                        elseif ($WholeWord) {
                            if ($WholeWordExcludeRegex[$xf].IsMatch($line)) { $isExcludedLine = $true; break }
                        }
                        else {
                            if ($line.IndexOf($xf, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) { $isExcludedLine = $true; break }
                        }
                    }
                    if ($isExcludedLine) {
                        if ($ExcludeScope -eq 'File') { $fileHasExcludeMatch = $true }
                        continue
                    }
                }
            }

            $matchedFilters = New-Object System.Collections.Generic.List[string]
            $candidateLine = $true
            if ($CombinedFilterRegex) { $candidateLine = $CombinedFilterRegex.IsMatch($line) }

            if ($candidateLine) {
                foreach ($f in $Filter) {
                    $isHit = $false
                    if ($UseRegex) {
                        try {
                            if ($CompiledFilterRegex -and $CompiledFilterRegex.ContainsKey($f) -and $CompiledFilterRegex[$f].IsMatch($line)) { $isHit = $true }
                        }
                        catch { }
                    }
                    elseif ($WholeWord) {
                        if ($WholeWordFilterRegex[$f].IsMatch($line)) { $isHit = $true }
                    }
                    else {
                        if ($line.IndexOf($f, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) { $isHit = $true }
                    }
                    if ($isHit) {
                        $matchedFilters.Add($f)
                        [void]$fileMatchedFilterSet.Add($f)
                    }
                }
            }

            if ($matchedFilters.Count -gt 0) {
                $before = if ($i -gt 0) { $lines[$i - 1] } else { $null }
                $after = if ($i -lt $lines.Count - 1) { $lines[$i + 1] } else { $null }
                $fileResults.Add([PSCustomObject]@{
                    LineNumber     = $i + 1
                    Before         = $before
                    MatchLine      = $line
                    After          = $after
                    MatchedFilters = $matchedFilters.ToArray()
                })
            }
        }

        if ($ExcludeScope -eq 'File' -and $fileHasExcludeMatch) {
            $result.Status = 'ExcludedFile'
            return $result
        }

        $passesMode = $true
        if ($MatchMode -in @('AllInFile', 'Proximity')) {
            foreach ($f in $Filter) {
                if (-not $fileMatchedFilterSet.Contains($f)) { $passesMode = $false; break }
            }
        }

        $minRangeForFile = $null
        if ($passesMode -and $MatchMode -eq 'Proximity') {
            $filterLineLists = @{}
            foreach ($r in $fileResults) {
                foreach ($mf in $r.MatchedFilters) {
                    if (-not $filterLineLists.ContainsKey($mf)) { $filterLineLists[$mf] = New-Object System.Collections.Generic.List[int] }
                    $filterLineLists[$mf].Add($r.LineNumber)
                }
            }
            foreach ($k2 in @($filterLineLists.Keys)) {
                $filterLineLists[$k2] = @($filterLineLists[$k2] | Sort-Object -Unique)
            }
            $minRangeForFile = Get-MinLineRangeAcrossFilters -FilterLineLists $filterLineLists -Filters $Filter
            if ($minRangeForFile -gt $ProximityLines) { $passesMode = $false }
        }

        if ($fileResults.Count -eq 0) {
            $result.Status = 'NoHit'
            return $result
        }

        if (-not $passesMode) {
            $result.Status = 'ModeExcluded'
            return $result
        }

        $result.Status = 'Hit'
        $result.Hits = $fileResults.ToArray()
        $result.TotalLineCount = $lines.Count
        $result.LinesCache = $lines
        if ($lines.Count -gt $MaxEmbedLines) {
            $result.LinesCache = $lines[0..($MaxEmbedLines - 1)]
        }
        $result.ProximityMinRange = $minRangeForFile

        return $result
    }
    catch {
        $result.Status = 'UnexpectedError'
        $result.ErrorMessage = $_.Exception.Message
        return $result
    }
}

function Add-FileBlockHtml {
    <#
        Appends one collapsible file entry (name/hit-count summary, expand-in-
        place body with Created/Modified dates, per-filter hit-count breakdown,
        any AllInFile/Proximity/PDF-confidence note, full extracted text with
        hits highlighted, and a fallback list for any hits beyond the embedded
        preview) to the report StringBuilder. Shared by both the flat and the
        Year/Month grouped layouts. Also emits an anchor id for the table of
        contents to link to.
    #>
    param(
        [System.Text.StringBuilder]$Sb,
        [Parameter(Mandatory = $true)]$FileGroup,
        [hashtable]$LinesCache,
        [hashtable]$LineTotalCount,
        [hashtable]$Meta,
        [hashtable]$LowConfidence,
        [bool]$RegexMode,
        [bool]$WholeWordMode,
        [string]$MatchModeLabel,
        [string[]]$AllFilters,
        [int]$ProximityLinesSetting,
        [hashtable]$ProximityInfo,
        [string]$AnchorId
    )

    $filePath = $FileGroup.Name
    $uri = Get-FileUri -Path $filePath
    $safePath = ConvertTo-HtmlSafe $filePath
    $hitCount = @($FileGroup.Group).Count
    $hitWord = if ($hitCount -eq 1) { 'hit' } else { 'hits' }

    $hitLineMap = @{}
    foreach ($hit in $FileGroup.Group) {
        $hitLineMap[$hit.LineNumber] = $hit.MatchedFilters
    }

    $cachedLines = $LinesCache[$filePath]
    $totalLines = $LineTotalCount[$filePath]
    $embeddedLineCount = if ($cachedLines) { $cachedLines.Count } else { 0 }
    $truncated = $totalLines -gt $embeddedLineCount

    $metaEntry = $Meta[$filePath]
    $createdStr = if ($metaEntry -and $metaEntry.Created) { $metaEntry.Created.ToString('yyyy-MM-dd HH:mm') } else { 'unknown' }
    $modifiedStr = if ($metaEntry -and $metaEntry.Modified) { $metaEntry.Modified.ToString('yyyy-MM-dd HH:mm') } else { 'unknown' }

    $anchorAttr = if ($AnchorId) { " id=""$AnchorId""" } else { '' }
    [void]$Sb.AppendLine("<details class=""file-block""$anchorAttr>")
    [void]$Sb.AppendLine("<summary><span class=""file-header-text"">$safePath</span> <span class=""lineno"">($hitCount $hitWord)</span></summary>")
    [void]$Sb.AppendLine('<div class="expanded-body">')

    if ($uri) {
        [void]$Sb.AppendLine("<p><a class=""filelink"" href=""$uri"">Open original file &#8599;</a> <span class=""file-path-text"">$safePath</span></p>")
    }
    else {
        [void]$Sb.AppendLine("<p class=""file-path-text"">$safePath</p>")
    }

    [void]$Sb.AppendLine("<p class=""meta-line"">Created: $createdStr &nbsp;|&nbsp; Modified: $modifiedStr</p>")

    if ($LowConfidence -and $LowConfidence.ContainsKey($filePath) -and $LowConfidence[$filePath]) {
        [void]$Sb.AppendLine('<p class="confidence-note">This PDF''s extracted text looks unreliable (often a sign of embedded/subsetted fonts) - if you expected more hits here, open the file directly to check manually.</p>')
    }

    if ($MatchModeLabel -eq 'AllInFile') {
        $filterListSafe = ($AllFilters | ForEach-Object { ConvertTo-HtmlSafe $_ }) -join ', '
        [void]$Sb.AppendLine("<p class=""truncate-note"">All required filters were found somewhere in this file: $filterListSafe</p>")
    }
    elseif ($MatchModeLabel -eq 'Proximity') {
        $filterListSafe = ($AllFilters | ForEach-Object { ConvertTo-HtmlSafe $_ }) -join ', '
        $mrText = if ($ProximityInfo -and $ProximityInfo.ContainsKey($filePath)) { "$($ProximityInfo[$filePath]) line(s)" } else { 'unknown' }
        [void]$Sb.AppendLine("<p class=""truncate-note"">All required filters found within $mrText of each other (limit: $ProximityLinesSetting): $filterListSafe</p>")
    }

    $perFilterCounts = @{}
    foreach ($hit in $FileGroup.Group) {
        foreach ($mf in $hit.MatchedFilters) {
            if (-not $perFilterCounts.ContainsKey($mf)) { $perFilterCounts[$mf] = 0 }
            $perFilterCounts[$mf] = $perFilterCounts[$mf] + 1
        }
    }
    if ($perFilterCounts.Count -gt 0) {
        $parts = foreach ($f in $AllFilters) {
            if ($perFilterCounts.ContainsKey($f)) {
                "$(ConvertTo-HtmlSafe $f): $($perFilterCounts[$f])"
            }
        }
        if ($parts) {
            [void]$Sb.AppendLine("<p class=""meta-line""><strong>Hits by filter:</strong> $($parts -join ' &nbsp;|&nbsp; ')</p>")
        }
    }

    if ($truncated) {
        [void]$Sb.AppendLine("<p class=""truncate-note"">Showing lines 1-$embeddedLineCount of $totalLines total extracted lines below. Open the original file to see the rest.</p>")
    }

    if ($cachedLines -and $cachedLines.Count -gt 0) {
        [void]$Sb.AppendLine('<pre class="full-file">')
        for ($ln = 1; $ln -le $cachedLines.Count; $ln++) {
            $rawLine = $cachedLines[$ln - 1]
            $numPrefix = ConvertTo-HtmlSafe ('{0,6}: ' -f $ln)
            if ($hitLineMap.ContainsKey($ln)) {
                $formatted = Format-MatchLine -Line $rawLine -MatchedFilters $hitLineMap[$ln] -RegexMode:$RegexMode -WholeWordMode:$WholeWordMode
                [void]$Sb.AppendLine("<span class=""hitline"">$numPrefix$formatted</span>")
            }
            else {
                [void]$Sb.AppendLine("$numPrefix$(ConvertTo-HtmlSafe $rawLine)")
            }
        }
        [void]$Sb.AppendLine('</pre>')
    }

    $beyondHits = $FileGroup.Group | Where-Object { $_.LineNumber -gt $embeddedLineCount }
    if ($beyondHits) {
        [void]$Sb.AppendLine('<p class="truncate-note">Additional hit(s) beyond the shown preview:</p>')
        foreach ($hit in $beyondHits) {
            [void]$Sb.AppendLine('<div class="hit">')
            [void]$Sb.AppendLine("<div class=""lineno"">Line $($hit.LineNumber)</div>")
            if ($null -ne $hit.Before) {
                [void]$Sb.AppendLine("<pre class=""context before"">$(ConvertTo-HtmlSafe $hit.Before)</pre>")
            }
            $formattedMatch = Format-MatchLine -Line $hit.MatchLine -MatchedFilters $hit.MatchedFilters -RegexMode:$RegexMode -WholeWordMode:$WholeWordMode
            [void]$Sb.AppendLine("<pre class=""context matchline"">$formattedMatch</pre>")
            if ($null -ne $hit.After) {
                [void]$Sb.AppendLine("<pre class=""context after"">$(ConvertTo-HtmlSafe $hit.After)</pre>")
            }
            [void]$Sb.AppendLine('</div>')
        }
    }

    [void]$Sb.AppendLine('</div>')
    [void]$Sb.AppendLine('</details>')
}

# ----------------------------------------------------------------------------
# Validate inputs
# ----------------------------------------------------------------------------

$SearchPath = Remove-SurroundingQuotes $SearchPath
$OutputFolder = Remove-SurroundingQuotes $OutputFolder
if ($OutputName) { $OutputName = Remove-SurroundingQuotes $OutputName }
if ($CacheFile) { $CacheFile = Remove-SurroundingQuotes $CacheFile }

$Filter = $Filter | Where-Object { $_ -and $_.Trim().Length -gt 0 }
if (-not $Filter -or $Filter.Count -eq 0) {
    throw "At least one non-empty -Filter value is required."
}

if ($ExcludeFilter) {
    $ExcludeFilter = @($ExcludeFilter | Where-Object { $_ -and $_.Trim().Length -gt 0 })
}
else {
    $ExcludeFilter = @()
}

if ($UseRegex) {
    $validFilters = New-Object System.Collections.Generic.List[string]
    foreach ($f in $Filter) {
        try {
            [void][regex]::new($f)
            $validFilters.Add($f)
        }
        catch {
            Write-Warning "Filter '$f' is not a valid regular expression and will be ignored: $($_.Exception.Message)"
        }
    }
    $Filter = $validFilters.ToArray()
    if ($Filter.Count -eq 0) {
        throw "No valid regex -Filter values remain after validation."
    }

    if ($ExcludeFilter.Count -gt 0) {
        $validExcludes = New-Object System.Collections.Generic.List[string]
        foreach ($f in $ExcludeFilter) {
            try {
                [void][regex]::new($f)
                $validExcludes.Add($f)
            }
            catch {
                Write-Warning "ExcludeFilter '$f' is not a valid regular expression and will be ignored: $($_.Exception.Message)"
            }
        }
        $ExcludeFilter = $validExcludes.ToArray()
    }
}

if ($Parallel -and $PSVersionTable.PSVersion.Major -lt 7) {
    Write-Warning "-Parallel requires PowerShell 7+. This session is running $($PSVersionTable.PSVersion) - continuing sequentially."
    $Parallel = $false
}

$wholeWordFilterRegex = @{}
$wholeWordExcludeRegex = @{}
$compiledFilterRegex = @{}
$compiledExcludeRegex = @{}

if ($WholeWord -and -not $UseRegex) {
    foreach ($f in $Filter) {
        $wholeWordFilterRegex[$f] = [regex]::new('\b' + [regex]::Escape($f) + '\b', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
    }
    foreach ($f in $ExcludeFilter) {
        $wholeWordExcludeRegex[$f] = [regex]::new('\b' + [regex]::Escape($f) + '\b', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
    }
}
if ($UseRegex) {
    foreach ($f in $Filter) {
        $compiledFilterRegex[$f] = [regex]::new($f, 'IgnoreCase, Compiled')
    }
    foreach ($f in $ExcludeFilter) {
        $compiledExcludeRegex[$f] = [regex]::new($f, 'IgnoreCase, Compiled')
    }
}

$combinedFilterRegex = Build-CombinedRegex -Filters $Filter -UseRegex $UseRegex.IsPresent -WholeWord $WholeWord.IsPresent
$combinedExcludeRegex = if ($ExcludeFilter.Count -gt 0) { Build-CombinedRegex -Filters $ExcludeFilter -UseRegex $UseRegex.IsPresent -WholeWord $WholeWord.IsPresent } else { $null }

try {
    $resolvedSearchPath = (Resolve-Path -LiteralPath $SearchPath -ErrorAction Stop).ProviderPath
}
catch {
    throw "SearchPath '$SearchPath' could not be found or is not accessible. Nothing was searched, nothing was written."
}
if (-not (Test-Path -LiteralPath $resolvedSearchPath -PathType Container)) {
    throw "SearchPath '$resolvedSearchPath' is not a folder."
}

if (-not (Test-Path -LiteralPath $OutputFolder -PathType Container)) {
    try {
        New-Item -ItemType Directory -LiteralPath $OutputFolder -Force | Out-Null
    }
    catch {
        throw "Could not create OutputFolder '$OutputFolder': $($_.Exception.Message)"
    }
}
$resolvedOutputFolder = (Resolve-Path -LiteralPath $OutputFolder -ErrorAction Stop).ProviderPath

if (-not $OutputName) {
    $OutputName = "SearchResults_{0}.html" -f (Get-Date -Format 'yyyyMMdd_HHmmss')
}
if ($OutputName -notmatch '\.html?$') {
    $OutputName = "$OutputName.html"
}
$outputFile = Join-Path $resolvedOutputFolder $OutputName

if ($resolvedOutputFolder.TrimEnd('\') -like "$($resolvedSearchPath.TrimEnd('\'))*") {
    Write-Warning "OutputFolder is inside SearchPath. The report file itself may show up in a future search of this same folder. This is harmless but you may prefer an OutputFolder outside SearchPath."
}

$searchAllExtensions = ($Extensions.Count -eq 1 -and $Extensions[0] -eq '*')
$extensionSet = New-Object 'System.Collections.Generic.HashSet[string]'([StringComparer]::OrdinalIgnoreCase)
foreach ($e in $Extensions) {
    $ext = if ($e.StartsWith('.')) { $e } else { ".$e" }
    [void]$extensionSet.Add($ext)
}

$maxBytes = [long]($MaxFileSizeMB * 1MB)

# ----------------------------------------------------------------------------
# Enumerate candidate files (read-only; permission errors are logged, not fatal;
# symlink/junction cycles are guarded against via resolved-path tracking)
# ----------------------------------------------------------------------------

Write-Host "Scanning '$resolvedSearchPath' ..." -ForegroundColor Cyan

$enumErrorCount = 0
$allFiles = Get-FilesSafely -RootPath $resolvedSearchPath -IncludeHidden:$IncludeHidden -EnumErrorCount ([ref]$enumErrorCount)

$candidateFiles = New-Object System.Collections.Generic.List[System.IO.FileInfo]
$skippedExt = 0
$skippedExcluded = 0

foreach ($f in $allFiles) {
    if ($ExcludeFolder.Count -gt 0) {
        $isExcluded = $false
        foreach ($ex in $ExcludeFolder) {
            if ($f.FullName -like "*$ex*") { $isExcluded = $true; break }
        }
        if ($isExcluded) { $skippedExcluded++; continue }
    }
    if (-not $searchAllExtensions -and -not $extensionSet.Contains($f.Extension)) {
        $skippedExt++
        continue
    }
    $candidateFiles.Add($f)
}

Write-Host "Found $($candidateFiles.Count) candidate file(s) to search (skipped $skippedExt by extension, $skippedExcluded by folder-exclude filter)." -ForegroundColor Cyan
if ($enumErrorCount -gt 0) {
    Write-Host "Note: $enumErrorCount folder(s)/item(s) could not be listed (likely permissions) and were skipped." -ForegroundColor Yellow
}

# ----------------------------------------------------------------------------
# Dry run: report what would be searched and stop, without reading any content
# ----------------------------------------------------------------------------

if ($DryRun) {
    Write-Host ''
    Write-Host '=== DRY RUN - no files were read, no report was written ===' -ForegroundColor Yellow
    $byExt = $candidateFiles | Group-Object Extension | Sort-Object Count -Descending
    foreach ($g in $byExt) {
        $label = if ($g.Name) { $g.Name } else { '(no extension)' }
        Write-Host ("  {0,-12} {1,6} file(s)" -f $label, $g.Count)
    }
    $totalMB = [Math]::Round((($candidateFiles | Measure-Object Length -Sum).Sum / 1MB), 1)
    Write-Host ''
    Write-Host "Total: $($candidateFiles.Count) file(s), approximately $totalMB MB, would be searched for: $($Filter -join ', ')" -ForegroundColor Cyan
    return
}

# ----------------------------------------------------------------------------
# Incremental cache: load prior results for files whose size/modified time and
# the search fingerprint (filters/mode/etc) haven't changed since last time
# ----------------------------------------------------------------------------

$cacheFingerprint = ([ordered]@{
    Filter        = $Filter
    ExcludeFilter = $ExcludeFilter
    MatchMode     = $MatchMode
    Proximity     = $ProximityLines
    ExcludeScope  = $ExcludeScope
    WholeWord     = $WholeWord.IsPresent
    UseRegex      = $UseRegex.IsPresent
} | ConvertTo-Json -Compress)

$priorCache = @{}
if ($CacheFile -and (Test-Path -LiteralPath $CacheFile -PathType Leaf)) {
    try {
        $raw = Get-Content -LiteralPath $CacheFile -Raw -ErrorAction Stop
        $parsed = $raw | ConvertFrom-Json -ErrorAction Stop
        if ($parsed.Fingerprint -eq $cacheFingerprint -and $parsed.Entries) {
            foreach ($prop in $parsed.Entries.PSObject.Properties) {
                $priorCache[$prop.Name] = $prop.Value
            }
            Write-Host "Loaded cache: $($priorCache.Count) prior file result(s) available for reuse." -ForegroundColor Cyan
        }
        else {
            Write-Host "Cache exists but filters/settings changed since last run - starting fresh." -ForegroundColor Yellow
        }
    }
    catch {
        Write-Host "Note: could not read cache file ($($_.Exception.Message)) - starting fresh." -ForegroundColor Yellow
    }
}

$toProcess = New-Object System.Collections.Generic.List[System.IO.FileInfo]
$reusedResults = New-Object System.Collections.Generic.List[object]
$cacheHits = 0

foreach ($f in $candidateFiles) {
    $cached = $priorCache[$f.FullName]
    if ($cached -and $cached.Length -eq $f.Length -and $cached.LastWriteTimeTicks -eq $f.LastWriteTime.Ticks) {
        $cacheHits++
        $reusedResults.Add([PSCustomObject]@{
            FullName          = $f.FullName
            Status            = $cached.Status
            Hits              = @($cached.Hits | ForEach-Object {
                [PSCustomObject]@{
                    LineNumber     = $_.LineNumber
                    Before         = $_.Before
                    MatchLine      = $_.MatchLine
                    After          = $_.After
                    MatchedFilters = @($_.MatchedFilters)
                }
            })
            Created           = [datetime]$cached.Created
            Modified          = [datetime]$cached.Modified
            LinesCache        = @($cached.LinesCache)
            TotalLineCount    = $cached.TotalLineCount
            ProximityMinRange = $cached.ProximityMinRange
            LowConfidencePdf  = [bool]$cached.LowConfidencePdf
            ErrorMessage      = $cached.ErrorMessage
        })
    }
    else {
        $toProcess.Add($f)
    }
}

if ($CacheFile -and $cacheHits -gt 0) {
    Write-Host "$cacheHits file(s) unchanged since last run - reused from cache, $($toProcess.Count) file(s) will be freshly searched." -ForegroundColor Cyan
}

# ----------------------------------------------------------------------------
# Search each file needing fresh processing (read-only) - sequential or
# parallel depending on -Parallel / PowerShell version
# ----------------------------------------------------------------------------

$freshResults = New-Object System.Collections.Generic.List[object]

if ($toProcess.Count -gt 0) {
    if ($Parallel) {
        Write-Host "Searching $($toProcess.Count) file(s) in parallel (throttle: $ThrottleLimit) ..." -ForegroundColor Cyan

        $helperNames = @(
            'ConvertTo-HtmlSafe', 'Get-TextFromBytes', 'Read-FileBytesRobust', 'Test-IsProbablyBinaryBytes',
            'Get-DocxPlainText', 'Get-PptxPlainText', 'ConvertFrom-Ascii85', 'Get-PdfPlainText',
            'Test-PdfExtractionLooksReliable', 'Get-RtfPlainText', 'Get-MinLineRangeAcrossFilters',
            'Invoke-SingleFileSearch'
        )
        $helperSource = ($helperNames | ForEach-Object { "function $_ {`n$(Get-Content "function:$_")`n}" }) -join "`n`n"

        $freshResults = $toProcess | ForEach-Object -ThrottleLimit $ThrottleLimit -Parallel {
            . ([scriptblock]::Create($using:helperSource))
            Invoke-SingleFileSearch -File $_ `
                -Filter $using:Filter -ExcludeFilter $using:ExcludeFilter `
                -MatchMode $using:MatchMode -ExcludeScope $using:ExcludeScope -ProximityLines $using:ProximityLines `
                -UseRegex $using:UseRegex.IsPresent -WholeWord $using:WholeWord.IsPresent -MaxEmbedLines $using:MaxEmbedLines `
                -PdfTimeoutSeconds $using:PdfTimeoutSeconds -FileTimeoutSeconds $using:FileTimeoutSeconds `
                -MaxRetries $using:MaxRetries -RetryDelayMs $using:RetryDelayMs `
                -CombinedFilterRegex $using:combinedFilterRegex -CombinedExcludeRegex $using:combinedExcludeRegex `
                -WholeWordFilterRegex $using:wholeWordFilterRegex -WholeWordExcludeRegex $using:wholeWordExcludeRegex `
                -CompiledFilterRegex $using:compiledFilterRegex -CompiledExcludeRegex $using:compiledExcludeRegex
        }
    }
    else {
        $totalCandidates = $toProcess.Count
        $progressCounter = 0
        $progressLastUpdate = Get-Date
        $liveHitCount = 0

        foreach ($file in $toProcess) {
            $progressCounter++
            if (((Get-Date) - $progressLastUpdate).TotalMilliseconds -ge 100) {
                $pct = [int](($progressCounter / $totalCandidates) * 100)
                Write-Progress -Activity 'Searching files' -Status "$progressCounter of $totalCandidates - $liveHitCount hit(s) so far - $($file.Name)" -PercentComplete $pct
                $progressLastUpdate = Get-Date
            }

            if ($file.Length -gt $maxBytes) {
                $freshResults.Add([PSCustomObject]@{ FullName = $file.FullName; Status = 'TooLarge'; Hits = @(); Created = $file.CreationTime; Modified = $file.LastWriteTime; LinesCache = @(); TotalLineCount = 0; ProximityMinRange = $null; LowConfidencePdf = $false; ErrorMessage = $null })
                continue
            }

            $r = Invoke-SingleFileSearch -File $file `
                -Filter $Filter -ExcludeFilter $ExcludeFilter `
                -MatchMode $MatchMode -ExcludeScope $ExcludeScope -ProximityLines $ProximityLines `
                -UseRegex $UseRegex.IsPresent -WholeWord $WholeWord.IsPresent -MaxEmbedLines $MaxEmbedLines `
                -PdfTimeoutSeconds $PdfTimeoutSeconds -FileTimeoutSeconds $FileTimeoutSeconds `
                -MaxRetries $MaxRetries -RetryDelayMs $RetryDelayMs `
                -CombinedFilterRegex $combinedFilterRegex -CombinedExcludeRegex $combinedExcludeRegex `
                -WholeWordFilterRegex $wholeWordFilterRegex -WholeWordExcludeRegex $wholeWordExcludeRegex `
                -CompiledFilterRegex $compiledFilterRegex -CompiledExcludeRegex $compiledExcludeRegex

            $freshResults.Add($r)
            if ($r.Status -eq 'Hit') { $liveHitCount += @($r.Hits).Count }
        }
        Write-Progress -Activity 'Searching files' -Completed
    }
}

# Normalize $freshResults into a definite array regardless of source: a
# List[object] from the sequential path (note: @() cannot safely wrap a
# List[object] in this PowerShell version - a confirmed runtime quirk - so
# .ToArray() is used instead), or whatever ForEach-Object -Parallel produced,
# which can itself be a single unwrapped item when only one file was processed.
$freshResultsArray = @()
if ($freshResults -is [System.Collections.Generic.List[object]]) {
    $freshResultsArray = $freshResults.ToArray()
}
elseif ($null -eq $freshResults) {
    $freshResultsArray = @()
}
elseif ($freshResults -is [array]) {
    $freshResultsArray = $freshResults
}
else {
    $freshResultsArray = @($freshResults)
}

# Still enforce the size cap for files pulled in via the parallel path too
# (kept simple: parallel path already only receives $toProcess, and
# Invoke-SingleFileSearch itself doesn't re-check size, so pre-filter here).
if ($Parallel -and $toProcess.Count -gt 0) {
    $tooLargeSet = New-Object 'System.Collections.Generic.HashSet[string]'([StringComparer]::OrdinalIgnoreCase)
    foreach ($f in $toProcess) {
        if ($f.Length -gt $maxBytes) { [void]$tooLargeSet.Add($f.FullName) }
    }
    if ($tooLargeSet.Count -gt 0) {
        $freshResultsArray = @($freshResultsArray | Where-Object { -not $tooLargeSet.Contains($_.FullName) })
        foreach ($fn in $tooLargeSet) {
            $fi = $toProcess | Where-Object { $_.FullName -eq $fn } | Select-Object -First 1
            $freshResultsArray += [PSCustomObject]@{ FullName = $fn; Status = 'TooLarge'; Hits = @(); Created = $fi.CreationTime; Modified = $fi.LastWriteTime; LinesCache = @(); TotalLineCount = 0; ProximityMinRange = $null; LowConfidencePdf = $false; ErrorMessage = $null }
        }
    }
}

$allFileResults = $reusedResults.ToArray() + $freshResultsArray

# ----------------------------------------------------------------------------
# Save cache (prune to only files that still exist as candidates)
# ----------------------------------------------------------------------------

if ($CacheFile) {
    $cacheEntries = @{}
    $candidateByPath = @{}
    foreach ($f in $candidateFiles) { $candidateByPath[$f.FullName] = $f }

    foreach ($r in $allFileResults) {
        $fi = $candidateByPath[$r.FullName]
        if (-not $fi) { continue }
        $cacheEntries[$r.FullName] = @{
            Length             = $fi.Length
            LastWriteTimeTicks = $fi.LastWriteTime.Ticks
            Status             = $r.Status
            Hits               = $r.Hits
            Created            = $r.Created.ToString('o')
            Modified           = $r.Modified.ToString('o')
            LinesCache         = $r.LinesCache
            TotalLineCount     = $r.TotalLineCount
            ProximityMinRange  = $r.ProximityMinRange
            LowConfidencePdf   = $r.LowConfidencePdf
            ErrorMessage       = $r.ErrorMessage
        }
    }

    try {
        @{ Fingerprint = $cacheFingerprint; Entries = $cacheEntries } | ConvertTo-Json -Depth 8 -Compress | Set-Content -LiteralPath $CacheFile -Encoding UTF8 -ErrorAction Stop
        Write-Host "Cache updated: $CacheFile ($($cacheEntries.Count) file(s) tracked)." -ForegroundColor Cyan
    }
    catch {
        Write-Host "Note: could not write cache file ($($_.Exception.Message))." -ForegroundColor Yellow
    }
}

# ----------------------------------------------------------------------------
# Aggregate results
# ----------------------------------------------------------------------------

$results = New-Object System.Collections.Generic.List[object]
$filesWithHits = New-Object System.Collections.Generic.HashSet[string]
$fileLinesCache = @{}
$fileLineTotalCount = @{}
$fileMeta = @{}
$fileProximityInfo = @{}
$fileLowConfidence = @{}

$filesSearched = 0
$skippedTooLarge = 0
$skippedBinary = 0
$skippedReadError = 0
$skippedByExclude = 0
$skippedByMode = 0
$skippedUnexpectedError = 0

foreach ($r in $allFileResults) {
    switch ($r.Status) {
        'TooLarge' { $skippedTooLarge++ }
        'Binary' { $skippedBinary++; $filesSearched++ }
        'ReadError' { $skippedReadError++ }
        'ExcludedFile' { $skippedByExclude++; $filesSearched++ }
        'ModeExcluded' { $skippedByMode++; $filesSearched++ }
        'UnexpectedError' {
            $skippedUnexpectedError++
            Write-Warning "Unexpected error processing '$($r.FullName)': $($r.ErrorMessage)"
        }
        'NoHit' { $filesSearched++ }
        'Hit' {
            $filesSearched++
            foreach ($h in $r.Hits) {
                $results.Add([PSCustomObject]@{
                    FullName       = $r.FullName
                    LineNumber     = $h.LineNumber
                    Before         = $h.Before
                    MatchLine      = $h.MatchLine
                    After          = $h.After
                    MatchedFilters = $h.MatchedFilters
                })
            }
            [void]$filesWithHits.Add($r.FullName)
            $fileMeta[$r.FullName] = [PSCustomObject]@{ Created = $r.Created; Modified = $r.Modified }
            $fileLineTotalCount[$r.FullName] = $r.TotalLineCount
            $fileLinesCache[$r.FullName] = $r.LinesCache
            $fileLowConfidence[$r.FullName] = $r.LowConfidencePdf
            if ($null -ne $r.ProximityMinRange) { $fileProximityInfo[$r.FullName] = $r.ProximityMinRange }
        }
    }
}

Write-Host "Searched $filesSearched file(s). Skipped: $skippedTooLarge too large, $skippedBinary binary, $skippedReadError unreadable/locked/unsupported, $skippedUnexpectedError with unexpected errors (see warnings above, if any)." -ForegroundColor Cyan
if ($ExcludeScope -eq 'File' -and $skippedByExclude -gt 0) {
    Write-Host "$skippedByExclude file(s) excluded entirely due to an ExcludeFilter match (ExcludeScope = File)." -ForegroundColor Yellow
}
if ($MatchMode -in @('AllInFile', 'Proximity') -and $skippedByMode -gt 0) {
    Write-Host "$skippedByMode file(s) had a partial match but were excluded (MatchMode = $MatchMode)." -ForegroundColor Yellow
}
Write-Host "Total hits: $($results.Count) across $($filesWithHits.Count) file(s)." -ForegroundColor Green

# ----------------------------------------------------------------------------
# Build the HTML report (in memory - nothing is written until the very end)
# ----------------------------------------------------------------------------

$sb = New-Object System.Text.StringBuilder

[void]$sb.AppendLine('<!DOCTYPE html>')
[void]$sb.AppendLine('<html lang="en"><head><meta charset="UTF-8">')
[void]$sb.AppendLine('<title>Text Search Report</title>')
[void]$sb.AppendLine(@'
<style>
  :root { --bg:#fafafa; --fg:#222; --panel-bg:#eef2f7; --panel-border:#ccd; --card-bg:#fff; --card-border:#ddd;
           --year-bg:#eef6ff; --year-border:#cfe0f2; --month-bg:#f5faff; --month-border:#dbe9f7;
           --muted:#666; --note-bg:#fff9db; --note-fg:#8a6300; --hit-bg:#fff9db; --mark-bg:#ff9800; --mark-fg:#1a1200;
           --link:#0b5fa5; --pre-bg:#fbfbfb; --pre-border:#eee; --bar-bg:#e3ecf5; --bar-fill:#0b5fa5; --confidence-bg:#fde8e8; --confidence-fg:#7a1f1f; }
  @media (prefers-color-scheme: dark) {
    :root { --bg:#1b1d21; --fg:#e6e6e6; --panel-bg:#242830; --panel-border:#3a3f4a; --card-bg:#20232a; --card-border:#3a3f4a;
             --year-bg:#1d2733; --year-border:#2c3b4d; --month-bg:#1a222c; --month-border:#28374a;
             --muted:#9aa4b2; --note-bg:#3a3320; --note-fg:#e0c060; --hit-bg:#3a3320; --mark-bg:#c97b13; --mark-fg:#fff3dc;
             --link:#6cb2f2; --pre-bg:#181a1f; --pre-border:#33373f; --bar-bg:#2c3441; --bar-fill:#6cb2f2; --confidence-bg:#3a2222; --confidence-fg:#f2a3a3; }
  }
  body { font-family: Segoe UI, Arial, sans-serif; margin: 2em; background: var(--bg); color: var(--fg); }
  h1 { font-size: 1.4em; }
  .summary { background: var(--panel-bg); border: 1px solid var(--panel-border); padding: 0.8em 1em; border-radius: 6px; margin-bottom: 1.5em; }
  .toc { background: var(--panel-bg); border: 1px solid var(--panel-border); padding: 0.8em 1em; border-radius: 6px; margin-bottom: 1.5em; max-height: 220px; overflow-y: auto; }
  .toc a { color: var(--link); text-decoration: none; display: inline-block; margin: 0.15em 0.6em 0.15em 0; }
  .toc a:hover { text-decoration: underline; }
  .bar-row { display: flex; align-items: center; margin: 0.2em 0; font-size: 0.9em; }
  .bar-label { width: 140px; flex-shrink: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-right: 0.5em; }
  .bar-track { flex-grow: 1; background: var(--bar-bg); border-radius: 3px; height: 14px; position: relative; }
  .bar-fill { background: var(--bar-fill); height: 100%; border-radius: 3px; }
  .bar-count { width: 3em; text-align: right; padding-left: 0.5em; flex-shrink: 0; }
  details.year-block { background: var(--year-bg); border: 1px solid var(--year-border); border-radius: 6px; margin-bottom: 1em; padding: 0.2em 1em; }
  details.year-block > summary.year-summary { cursor: pointer; padding: 0.6em 0; font-weight: 700; font-size: 1.1em; }
  details.month-block { background: var(--month-bg); border: 1px solid var(--month-border); border-radius: 6px; margin: 0.6em 0; padding: 0.2em 1em; }
  details.month-block > summary.month-summary { cursor: pointer; padding: 0.5em 0; font-weight: 600; }
  details.file-block { background: var(--card-bg); border: 1px solid var(--card-border); border-radius: 6px; margin-bottom: 0.8em; padding: 0.2em 1em; }
  details.file-block > summary { cursor: pointer; padding: 0.6em 0; font-weight: 600; }
  details.file-block > summary:hover { color: var(--link); }
  .file-header-text { word-break: break-all; }
  .expanded-body { padding: 0.4em 0 0.8em 0; border-top: 1px dashed var(--card-border); }
  .file-path-text { color: var(--muted); font-size: 0.85em; word-break: break-all; }
  .meta-line { color: var(--muted); font-size: 0.85em; }
  a.filelink { text-decoration: none; color: var(--link); font-weight: 600; }
  a.filelink:hover { text-decoration: underline; }
  .lineno { color: var(--muted); font-size: 0.85em; font-weight: normal; }
  .truncate-note { color: var(--note-fg); background: var(--note-bg); padding: 0.4em 0.6em; border-radius: 4px; font-size: 0.9em; }
  .confidence-note { color: var(--confidence-fg); background: var(--confidence-bg); padding: 0.4em 0.6em; border-radius: 4px; font-size: 0.9em; }
  pre.full-file { white-space: pre-wrap; word-break: break-word; font-family: Consolas, monospace; font-size: 0.9em; background: var(--pre-bg); border: 1px solid var(--pre-border); padding: 0.6em; border-radius: 4px; max-height: 70vh; overflow-y: auto; color: var(--fg); }
  span.hitline { display: block; background: var(--hit-bg); border-left: 3px solid #e0b400; padding-left: 0.2em; }
  mark { background: var(--mark-bg); color: var(--mark-fg); font-weight: 700; padding: 0 3px; border-radius: 2px; }
  .hit { border-top: 1px dashed var(--card-border); padding: 0.5em 0; }
  pre.context { margin: 0.15em 0; padding: 0.15em 0.4em; white-space: pre-wrap; word-break: break-word; font-family: Consolas, monospace; color: var(--fg); }
  pre.before, pre.after { color: var(--muted); }
  pre.matchline { background: var(--hit-bg); border-left: 3px solid #e0b400; }
  .no-hits { color: var(--muted); font-style: italic; }
</style>
'@)
[void]$sb.AppendLine('</head><body>')
[void]$sb.AppendLine('<h1>Text Search Report</h1>')

$filterListHtml = ($Filter | ForEach-Object { ConvertTo-HtmlSafe $_ }) -join ', '
[void]$sb.AppendLine('<div class="summary">')
[void]$sb.AppendLine("<div><strong>Search folder:</strong> $(ConvertTo-HtmlSafe $resolvedSearchPath)</div>")
[void]$sb.AppendLine("<div><strong>Filters:</strong> $filterListHtml $(if ($UseRegex) { '(regex mode)' }) $(if ($WholeWord -and -not $UseRegex) { '(whole word)' })</div>")
if ($ExcludeFilter.Count -gt 0) {
    $excludeListHtml = ($ExcludeFilter | ForEach-Object { ConvertTo-HtmlSafe $_ }) -join ', '
    [void]$sb.AppendLine("<div><strong>Excluding:</strong> $excludeListHtml (scope: $ExcludeScope)</div>")
}
[void]$sb.AppendLine("<div><strong>Match mode:</strong> $MatchMode$(if ($MatchMode -eq 'Proximity') { " (within $ProximityLines line(s))" }) &nbsp; <strong>Grouped by:</strong> $GroupBy$(if ($Parallel) { ' &nbsp; <strong>Parallel:</strong> yes' })</div>")
[void]$sb.AppendLine("<div><strong>Run time:</strong> $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')</div>")
[void]$sb.AppendLine("<div><strong>Files searched:</strong> $filesSearched &nbsp; <strong>Files with hits:</strong> $($filesWithHits.Count) &nbsp; <strong>Total hits:</strong> $($results.Count)</div>")

$skipNote = "$skippedExt by extension, $skippedExcluded by folder-exclude filter, $skippedTooLarge too large, $skippedBinary binary, $skippedReadError unreadable/locked/unsupported, $skippedUnexpectedError with unexpected errors"
if ($ExcludeScope -eq 'File' -and $skippedByExclude -gt 0) { $skipNote += ", $skippedByExclude excluded by ExcludeFilter" }
if ($MatchMode -in @('AllInFile', 'Proximity') -and $skippedByMode -gt 0) { $skipNote += ", $skippedByMode missing required filters ($MatchMode mode)" }
[void]$sb.AppendLine("<div><strong>Skipped:</strong> $skipNote</div>")

$aggregateCounts = @{}
foreach ($r in $results) {
    foreach ($mf in $r.MatchedFilters) {
        if (-not $aggregateCounts.ContainsKey($mf)) { $aggregateCounts[$mf] = 0 }
        $aggregateCounts[$mf] = $aggregateCounts[$mf] + 1
    }
}
if ($aggregateCounts.Count -gt 0) {
    $maxCount = ($aggregateCounts.Values | Measure-Object -Maximum).Maximum
    [void]$sb.AppendLine('<div style="margin-top:0.6em;"><strong>Hits by filter:</strong></div>')
    foreach ($f in $Filter) {
        $c = if ($aggregateCounts.ContainsKey($f)) { $aggregateCounts[$f] } else { 0 }
        $pct = if ($maxCount -gt 0) { [int](($c / $maxCount) * 100) } else { 0 }
        [void]$sb.AppendLine("<div class=""bar-row""><span class=""bar-label"" title=""$(ConvertTo-HtmlSafe $f)"">$(ConvertTo-HtmlSafe $f)</span><span class=""bar-track""><span class=""bar-fill"" style=""width:$pct%""></span></span><span class=""bar-count"">$c</span></div>")
    }
}

[void]$sb.AppendLine('<div style="margin-top:0.6em;">Click a file below to expand its content in this page. A separate small link inside lets you open the real file if you want it.</div>')
[void]$sb.AppendLine('</div>')

if ($results.Count -eq 0) {
    [void]$sb.AppendLine('<p class="no-hits">No matches found.</p>')
}
elseif ($GroupBy -eq 'None') {
    $grouped = $results | Group-Object FullName | Sort-Object Name

    if (@($grouped).Count -gt 3) {
        [void]$sb.AppendLine('<div class="toc"><strong>Jump to:</strong><br/>')
        $tocIdx = 0
        foreach ($g in $grouped) {
            $tocIdx++
            [void]$sb.AppendLine("<a href=""#file-$tocIdx"">$(ConvertTo-HtmlSafe (Split-Path -Leaf $g.Name))</a>")
        }
        [void]$sb.AppendLine('</div>')
    }

    $idx = 0
    foreach ($group in $grouped) {
        $idx++
        try {
            Add-FileBlockHtml -Sb $sb -FileGroup $group -LinesCache $fileLinesCache -LineTotalCount $fileLineTotalCount -Meta $fileMeta -LowConfidence $fileLowConfidence -RegexMode:$UseRegex -WholeWordMode:$WholeWord -MatchModeLabel $MatchMode -AllFilters $Filter -ProximityLinesSetting $ProximityLines -ProximityInfo $fileProximityInfo -AnchorId "file-$idx"
        }
        catch {
            [void]$sb.AppendLine("<p class=""truncate-note"">Could not render details for ""$(ConvertTo-HtmlSafe $group.Name)"": $(ConvertTo-HtmlSafe $_.Exception.Message)</p>")
        }
    }
}
else {
    $grouped = $results | Group-Object FullName | Sort-Object Name

    $enriched = foreach ($g in $grouped) {
        $m = $fileMeta[$g.Name]
        $dateField = if ($GroupBy -eq 'Created') { $m.Created } else { $m.Modified }
        [PSCustomObject]@{
            FileGroup = $g
            Date      = $dateField
            Year      = $dateField.Year
            MonthNum  = $dateField.Month
            MonthName = $dateField.ToString('MMMM')
        }
    }

    $byYear = $enriched | Group-Object Year | Sort-Object { [int]$_.Name } -Descending

    if (@($grouped).Count -gt 3) {
        [void]$sb.AppendLine('<div class="toc"><strong>Jump to:</strong><br/>')
        foreach ($yearGroup in $byYear) {
            [void]$sb.AppendLine("<a href=""#year-$($yearGroup.Name)"">$($yearGroup.Name)</a>")
        }
        [void]$sb.AppendLine('</div>')
    }

    $fileAnchorIdx = 0
    foreach ($yearGroup in $byYear) {
        $yearFileCount = $yearGroup.Count
        $yearHitCount = ($yearGroup.Group | ForEach-Object { @($_.FileGroup.Group).Count } | Measure-Object -Sum).Sum
        [void]$sb.AppendLine("<details class=""year-block"" id=""year-$($yearGroup.Name)""><summary class=""year-summary"">$($yearGroup.Name) &mdash; $yearFileCount file(s), $yearHitCount hit(s)</summary>")
        [void]$sb.AppendLine('<div class="year-body">')

        $byMonth = $yearGroup.Group | Group-Object MonthNum | Sort-Object { [int]$_.Name } -Descending
        foreach ($monthGroup in $byMonth) {
            $monthName = $monthGroup.Group[0].MonthName
            $monthFileCount = $monthGroup.Count
            $monthHitCount = ($monthGroup.Group | ForEach-Object { @($_.FileGroup.Group).Count } | Measure-Object -Sum).Sum
            [void]$sb.AppendLine("<details class=""month-block""><summary class=""month-summary"">$monthName &mdash; $monthFileCount file(s), $monthHitCount hit(s)</summary>")
            [void]$sb.AppendLine('<div class="month-body">')

            $sortedItems = $monthGroup.Group | Sort-Object Date -Descending
            foreach ($item in $sortedItems) {
                $fileAnchorIdx++
                try {
                    Add-FileBlockHtml -Sb $sb -FileGroup $item.FileGroup -LinesCache $fileLinesCache -LineTotalCount $fileLineTotalCount -Meta $fileMeta -LowConfidence $fileLowConfidence -RegexMode:$UseRegex -WholeWordMode:$WholeWord -MatchModeLabel $MatchMode -AllFilters $Filter -ProximityLinesSetting $ProximityLines -ProximityInfo $fileProximityInfo -AnchorId "file-$fileAnchorIdx"
                }
                catch {
                    [void]$sb.AppendLine("<p class=""truncate-note"">Could not render details for ""$(ConvertTo-HtmlSafe $item.FileGroup.Name)"": $(ConvertTo-HtmlSafe $_.Exception.Message)</p>")
                }
            }

            [void]$sb.AppendLine('</div>')
            [void]$sb.AppendLine('</details>')
        }

        [void]$sb.AppendLine('</div>')
        [void]$sb.AppendLine('</details>')
    }
}

[void]$sb.AppendLine('</body></html>')

# ----------------------------------------------------------------------------
# Write the HTML report (the primary write operation this script performs)
# ----------------------------------------------------------------------------

try {
    Set-Content -LiteralPath $outputFile -Value $sb.ToString() -Encoding UTF8 -ErrorAction Stop
}
catch {
    throw "Failed to write report to '$outputFile': $($_.Exception.Message)"
}

Write-Host "Report written to: $outputFile" -ForegroundColor Green

# ----------------------------------------------------------------------------
# Optional CSV / JSON export
# ----------------------------------------------------------------------------

if (($ExportCsv -or $ExportJson) -and $results.Count -eq 0) {
    Write-Host "No hits found - skipping CSV/JSON export." -ForegroundColor Yellow
}
elseif ($ExportCsv -or $ExportJson) {
    $exportRows = foreach ($r in $results) {
        $m = $fileMeta[$r.FullName]
        [PSCustomObject]@{
            FilePath       = $r.FullName
            LineNumber     = $r.LineNumber
            MatchedFilters = ($r.MatchedFilters -join '; ')
            Before         = $r.Before
            MatchLine      = $r.MatchLine
            After          = $r.After
            Created        = if ($m) { $m.Created } else { $null }
            Modified       = if ($m) { $m.Modified } else { $null }
        }
    }

    if ($ExportCsv) {
        $csvPath = Join-Path $resolvedOutputFolder ($OutputName -replace '\.html?$', '.csv')
        try {
            $exportRows | Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8 -ErrorAction Stop
            Write-Host "CSV export written to: $csvPath" -ForegroundColor Green
        }
        catch {
            Write-Warning "Failed to write CSV export to '$csvPath': $($_.Exception.Message)"
        }
    }

    if ($ExportJson) {
        $jsonPath = Join-Path $resolvedOutputFolder ($OutputName -replace '\.html?$', '.json')
        try {
            $exportRows | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $jsonPath -Encoding UTF8 -ErrorAction Stop
            Write-Host "JSON export written to: $jsonPath" -ForegroundColor Green
        }
        catch {
            Write-Warning "Failed to write JSON export to '$jsonPath': $($_.Exception.Message)"
        }
    }
}

if ($OpenReport) {
    try {
        Invoke-Item -LiteralPath $outputFile
    }
    catch {
        Write-Warning "Could not auto-open the report: $($_.Exception.Message)"
    }
}
