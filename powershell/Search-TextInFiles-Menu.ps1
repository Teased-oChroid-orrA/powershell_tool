<#
.SYNOPSIS
    Interactive menu front-end for Search-TextInFiles.ps1 - toggle options on/off,
    only get asked for input on the ones you turn on, then run the search.

.DESCRIPTION
    This menu does not search anything itself and never touches anything under
    -SearchPath. It only collects your settings, then calls Search-TextInFiles.ps1
    with the equivalent parameters - exactly as if you'd typed them yourself.

    INTERACTION MODES:
      - If your console supports it, use Up/Down arrows to move, Enter to
        edit/toggle the highlighted row, digits 1-9 to jump straight to one of
        the first nine rows, and Esc to quit.
      - If arrow-key input isn't available (redirected input, some remote
        sessions, certain hosts like the ISE), the menu automatically falls back
        to typing a row number plus P/R/Q for Preview/Run/Quit - fully
        functional either way, nothing to configure.

    FILES THIS SCRIPT WRITES:
      - Nothing under -SearchPath, ever.
      - One small settings file next to this menu script,
        ".search-textinfiles-menu.settings.json", so your last-used options are
        there the next time you open the menu. This can include your search
        terms and folder paths. Pass -NoSaveSettings to skip writing/reading it
        for a given session, or -ResetSettings to delete it and start fresh.
      - Whatever report(s) Search-TextInFiles.ps1 itself produces under the
        output folder you choose (unchanged from that script's own behavior).

.PARAMETER MainScriptPath
    Path to Search-TextInFiles.ps1. Defaults to a file of that name in the same
    folder as this menu script.

.PARAMETER NoSaveSettings
    Don't read or write the settings file this session - useful on a shared
    machine if you'd rather not leave search terms/paths on disk.

.PARAMETER ResetSettings
    Delete any saved settings file before starting, then proceed with defaults.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File .\Search-TextInFiles-Menu.ps1
#>

[CmdletBinding()]
param(
    [string]$MainScriptPath = (Join-Path $PSScriptRoot 'Search-TextInFiles.ps1'),
    [switch]$NoSaveSettings,
    [switch]$ResetSettings
)

if (-not (Test-Path -LiteralPath $MainScriptPath -PathType Leaf)) {
    Write-Host "Could not find Search-TextInFiles.ps1." -ForegroundColor Red
    Write-Host "Expected at: $MainScriptPath" -ForegroundColor Red
    Write-Host "Put both scripts in the same folder, or run this with:" -ForegroundColor Yellow
    Write-Host "    .\Search-TextInFiles-Menu.ps1 -MainScriptPath 'C:\path\to\Search-TextInFiles.ps1'" -ForegroundColor Yellow
    return
}

$script:SettingsPath = Join-Path $PSScriptRoot '.search-textinfiles-menu.settings.json'

if ($ResetSettings -and (Test-Path -LiteralPath $script:SettingsPath -PathType Leaf)) {
    try {
        Remove-Item -LiteralPath $script:SettingsPath -Force -ErrorAction Stop
        Write-Host "Cleared saved settings." -ForegroundColor Yellow
    }
    catch {
        Write-Host "Could not clear saved settings: $($_.Exception.Message)" -ForegroundColor Yellow
    }
}

# ----------------------------------------------------------------------------
# Small input helpers
# ----------------------------------------------------------------------------

function Remove-SurroundingQuotes {
    <#
        Read-Host returns exactly what you type, including quote characters -
        unlike a real command line, it does NOT strip them. Typing "C:\My Folder"
        (with quotes) at a prompt would otherwise be stored as a path that
        literally starts and ends with a " character, which never exists on disk.
        This strips one matching pair of leading/trailing quotes so pasted or
        quoted paths still work.
    #>
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

function Read-NonEmptyOrKeep {
    param([string]$Prompt, [string]$Default = '')
    $suffix = if ($Default) { " [$Default]" } else { ' [none]' }
    $val = Read-Host "$Prompt$suffix"
    if ([string]::IsNullOrWhiteSpace($val)) { return $Default }
    return Remove-SurroundingQuotes $val.Trim()
}

function Read-CommaList {
    <# Comma-separated values. Blank keeps current. Typing CLEAR empties it. #>
    param([string]$Prompt, [string[]]$Current)
    $currentDisplay = if ($Current -and @($Current).Count -gt 0) { $Current -join ', ' } else { '(none)' }
    Write-Host "Current: $currentDisplay"
    $val = Read-Host "$Prompt (comma-separated, blank = keep current, CLEAR = empty it)"
    if ([string]::IsNullOrWhiteSpace($val)) { return $Current }
    if ($val.Trim().ToUpperInvariant() -eq 'CLEAR') { return @() }
    return @($val -split ',' | ForEach-Object { Remove-SurroundingQuotes $_.Trim() } | Where-Object { $_ -ne '' })
}

function Read-IntValue {
    param([string]$Prompt, [int]$Default)
    while ($true) {
        $val = Read-Host "$Prompt [$Default]"
        if ([string]::IsNullOrWhiteSpace($val)) { return $Default }
        $parsed = 0
        if ([int]::TryParse($val.Trim(), [ref]$parsed)) { return $parsed }
        Write-Host "Please enter a whole number." -ForegroundColor Yellow
    }
}

function Read-DoubleValue {
    param([string]$Prompt, [double]$Default)
    while ($true) {
        $val = Read-Host "$Prompt [$Default]"
        if ([string]::IsNullOrWhiteSpace($val)) { return $Default }
        $parsed = 0.0
        if ([double]::TryParse($val.Trim(), [ref]$parsed)) { return $parsed }
        Write-Host "Please enter a number." -ForegroundColor Yellow
    }
}

function Read-ChoiceValue {
    param([string]$Prompt, [string[]]$Choices, [string]$Default)
    Write-Host "$Prompt - options: $($Choices -join ' / ')"
    while ($true) {
        $val = Read-Host "Choose [$Default]"
        if ([string]::IsNullOrWhiteSpace($val)) { return $Default }
        $match = $Choices | Where-Object { $_ -ieq $val.Trim() }
        if ($match) { return $match }
        Write-Host "Please enter one of: $($Choices -join ', ')" -ForegroundColor Yellow
    }
}

# ----------------------------------------------------------------------------
# Console capability detection
# ----------------------------------------------------------------------------

function Test-InteractiveConsole {
    <#
        Arrow-key navigation needs a real, non-redirected console. Redirected
        input (piped/scripted runs), some remote sessions, and hosts like the
        ISE either can't supply it or throw when asked - in every one of those
        cases we fall back to plain numbered input instead of failing.
    #>
    try {
        if ([Console]::IsInputRedirected) { return $false }
        $null = [Console]::KeyAvailable
        return $true
    }
    catch {
        return $false
    }
}

# ----------------------------------------------------------------------------
# Settings persistence (one small JSON file next to this script)
# ----------------------------------------------------------------------------

function Import-MenuSettings {
    param([System.Collections.Specialized.OrderedDictionary]$State)
    if (-not (Test-Path -LiteralPath $script:SettingsPath -PathType Leaf)) { return }
    try {
        $raw = Get-Content -LiteralPath $script:SettingsPath -Raw -ErrorAction Stop
        $saved = $raw | ConvertFrom-Json -ErrorAction Stop
        foreach ($key in @($State.Keys)) {
            if ($saved.PSObject.Properties.Name -contains $key) {
                $val = $saved.$key
                if ($key -in @('Filter', 'ExcludeFilter', 'ExcludeFolder')) {
                    $State[$key] = @($val | Where-Object { $null -ne $_ })
                }
                elseif ($key -eq 'Extensions') {
                    $State[$key] = if ($null -eq $val) { $null } else { @($val) }
                }
                else {
                    $State[$key] = $val
                }
            }
        }
    }
    catch {
        Write-Host "Note: couldn't load saved settings ($($_.Exception.Message)); starting with defaults." -ForegroundColor DarkYellow
        Start-Sleep -Milliseconds 1200
    }
}

function Save-MenuSettings {
    param([System.Collections.Specialized.OrderedDictionary]$State)
    if ($NoSaveSettings) { return }
    try {
        $State | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $script:SettingsPath -Encoding UTF8 -ErrorAction Stop
    }
    catch {
        Write-Host "Note: couldn't save settings for next time ($($_.Exception.Message))." -ForegroundColor DarkYellow
    }
}

# ----------------------------------------------------------------------------
# Settings state + menu structure
# ----------------------------------------------------------------------------

$state = [ordered]@{
    SearchPath        = ''
    OutputFolder      = ''
    OutputName        = ''
    Filter            = @()
    ExcludeFilter     = @()
    MatchMode         = 'AnyLine'
    ProximityLines    = 5
    ExcludeScope      = 'Line'
    WholeWord         = $false
    UseRegex          = $false
    GroupBy           = 'Created'
    Extensions        = $null   # $null means "use the main script's built-in default list"
    ExcludeFolder     = @()
    MaxFileSizeMB     = 50
    MaxEmbedLines     = 4000
    PdfTimeoutSeconds = 15
    IncludeHidden     = $false
    OpenReport        = $false
    ExportCsv         = $false
    ExportJson        = $false
    Parallel          = $false
    ThrottleLimit     = 5
    CacheFile         = ''
    DryRun            = $false
    MaxRetries        = 3
    RetryDelayMs      = 250
    FileTimeoutSeconds = 30
}

if (-not $NoSaveSettings) {
    Import-MenuSettings -State $state
}

$menuItems = @(
    @{ Id = 'SearchPath'; Section = 'Required'; Label = 'Search folder'; Type = 'Text'; Required = $true }
    @{ Id = 'OutputFolder'; Section = 'Required'; Label = 'Output folder'; Type = 'Text'; Required = $true }
    @{ Id = 'Filter'; Section = 'Required'; Label = 'Filters'; Type = 'List'; Required = $true }
    @{ Id = 'MatchMode'; Section = 'Matching'; Label = 'Match mode'; Type = 'Choice'; Choices = @('AnyLine', 'AllInFile', 'Proximity') }
    @{ Id = 'ProximityLines'; Section = 'Matching'; Label = 'Proximity lines'; Type = 'Int' }
    @{ Id = 'UseRegex'; Section = 'Matching'; Label = 'Use regex'; Type = 'Bool' }
    @{ Id = 'WholeWord'; Section = 'Matching'; Label = 'Whole word matching'; Type = 'Bool' }
    @{ Id = 'ExcludeFilter'; Section = 'Matching'; Label = 'Exclude filters'; Type = 'List' }
    @{ Id = 'ExcludeScope'; Section = 'Matching'; Label = 'Exclude scope'; Type = 'Choice'; Choices = @('Line', 'File') }
    @{ Id = 'Extensions'; Section = 'Scope and output'; Label = 'Extensions'; Type = 'ExtList' }
    @{ Id = 'ExcludeFolder'; Section = 'Scope and output'; Label = 'Exclude folders'; Type = 'List' }
    @{ Id = 'IncludeHidden'; Section = 'Scope and output'; Label = 'Include hidden files'; Type = 'Bool' }
    @{ Id = 'MaxFileSizeMB'; Section = 'Scope and output'; Label = 'Max file size (MB)'; Type = 'Double' }
    @{ Id = 'MaxEmbedLines'; Section = 'Scope and output'; Label = 'Max embedded lines'; Type = 'Int' }
    @{ Id = 'GroupBy'; Section = 'Scope and output'; Label = 'Group by'; Type = 'Choice'; Choices = @('Created', 'Modified', 'None') }
    @{ Id = 'OutputName'; Section = 'Scope and output'; Label = 'Output file name'; Type = 'Text' }
    @{ Id = 'OpenReport'; Section = 'Scope and output'; Label = 'Open report when done'; Type = 'Bool' }
    @{ Id = 'ExportCsv'; Section = 'Scope and output'; Label = 'Export CSV'; Type = 'Bool' }
    @{ Id = 'ExportJson'; Section = 'Scope and output'; Label = 'Export JSON'; Type = 'Bool' }
    @{ Id = 'Parallel'; Section = 'Performance and robustness'; Label = 'Parallel processing'; Type = 'Bool' }
    @{ Id = 'ThrottleLimit'; Section = 'Performance and robustness'; Label = 'Parallel throttle limit'; Type = 'Int' }
    @{ Id = 'CacheFile'; Section = 'Performance and robustness'; Label = 'Cache file (incremental re-scan)'; Type = 'Text' }
    @{ Id = 'DryRun'; Section = 'Performance and robustness'; Label = 'Dry run (list files only)'; Type = 'Bool' }
    @{ Id = 'PdfTimeoutSeconds'; Section = 'Performance and robustness'; Label = 'PDF extraction timeout (s)'; Type = 'Int' }
    @{ Id = 'FileTimeoutSeconds'; Section = 'Performance and robustness'; Label = 'Per-file read timeout (s)'; Type = 'Int' }
    @{ Id = 'MaxRetries'; Section = 'Performance and robustness'; Label = 'Max retries (locked files)'; Type = 'Int' }
    @{ Id = 'RetryDelayMs'; Section = 'Performance and robustness'; Label = 'Retry delay (ms)'; Type = 'Int' }
)

$actionItems = @(
    @{ Id = '__PREVIEW__'; Label = 'Preview the equivalent command' }
    @{ Id = '__RUN__'; Label = 'Run the search now' }
    @{ Id = '__QUIT__'; Label = 'Quit' }
)

# ----------------------------------------------------------------------------
# Rendering
# ----------------------------------------------------------------------------

function Get-ItemDisplayValue {
    param($Item, [System.Collections.Specialized.OrderedDictionary]$State)
    $val = $State[$Item.Id]
    switch ($Item.Type) {
        'Bool' {
            if ($val) { return @{ Text = 'On'; Color = 'Green' } }
            return @{ Text = 'Off'; Color = 'DarkGray' }
        }
        'List' {
            $arr = @($val)
            if ($arr.Count -eq 0) {
                if ($Item.Required) { return @{ Text = '<not set>'; Color = 'Red' } }
                return @{ Text = '(none)'; Color = 'DarkGray' }
            }
            return @{ Text = ($arr -join ', '); Color = 'White' }
        }
        'ExtList' {
            if ($null -eq $val) { return @{ Text = '(built-in default list)'; Color = 'DarkGray' } }
            $arr = @($val)
            if ($arr.Count -eq 0) { return @{ Text = '(none - will match nothing)'; Color = 'Red' } }
            return @{ Text = ($arr -join ', '); Color = 'White' }
        }
        default {
            if ([string]::IsNullOrWhiteSpace([string]$val)) {
                if ($Item.Required) { return @{ Text = '<not set>'; Color = 'Red' } }
                if ($Item.Id -eq 'OutputName') { return @{ Text = '(auto-generated)'; Color = 'DarkGray' } }
                return @{ Text = '(none)'; Color = 'DarkGray' }
            }
            return @{ Text = [string]$val; Color = 'White' }
        }
    }
}

function Get-ReadinessStatus {
    param([System.Collections.Specialized.OrderedDictionary]$State)
    $missing = @()
    if (-not $State.SearchPath) { $missing += 'Search folder' }
    if (-not $State.OutputFolder) { $missing += 'Output folder' }
    if (-not $State.Filter -or @($State.Filter).Count -eq 0) { $missing += 'Filters' }
    if ($missing.Count -eq 0) { return @{ Text = 'Ready to run'; Color = 'Green' } }
    return @{ Text = "Missing: $($missing -join ', ')"; Color = 'Red' }
}

function Show-MenuScreen {
    param(
        [System.Collections.Specialized.OrderedDictionary]$State,
        [array]$MenuItems,
        [array]$ActionItems,
        [int]$SelectedIndex = -1,
        [switch]$ArrowMode
    )
    try { Clear-Host } catch { }

    $width = 78
    Write-Host ('=' * $width) -ForegroundColor DarkCyan
    Write-Host '  SEARCH-TEXTINFILES - INTERACTIVE MENU' -ForegroundColor Cyan
    $ready = Get-ReadinessStatus -State $State
    Write-Host "  $($ready.Text)" -ForegroundColor $ready.Color
    Write-Host ('=' * $width) -ForegroundColor DarkCyan

    $lastSection = $null
    for ($i = 0; $i -lt $MenuItems.Count; $i++) {
        $item = $MenuItems[$i]
        if ($item.Section -ne $lastSection) {
            Write-Host ''
            Write-Host " $($item.Section)" -ForegroundColor Yellow
            $lastSection = $item.Section
        }

        $isSelected = ($ArrowMode -and $i -eq $SelectedIndex)
        $numLabel = "{0,2})" -f ($i + 1)
        $reqMark = if ($item.Required) { '*' } else { ' ' }
        $disp = Get-ItemDisplayValue -Item $item -State $State
        $labelPadded = $item.Label.PadRight(24)
        $prefix = if ($isSelected) { ' > ' } else { '   ' }
        $lineText = "$prefix$numLabel $reqMark$labelPadded : $($disp.Text)"

        if ($isSelected) {
            Write-Host $lineText -ForegroundColor Black -BackgroundColor Gray
        }
        else {
            Write-Host -NoNewline "$prefix$numLabel $reqMark$labelPadded : "
            Write-Host $disp.Text -ForegroundColor $disp.Color
        }
    }

    Write-Host ''
    Write-Host ' Actions' -ForegroundColor Yellow
    for ($j = 0; $j -lt $ActionItems.Count; $j++) {
        $globalIndex = $MenuItems.Count + $j
        $isSelected = ($ArrowMode -and $globalIndex -eq $SelectedIndex)
        $letter = @('P', 'R', 'Q')[$j]
        $prefix = if ($isSelected) { ' > ' } else { '   ' }
        $lineText = "$prefix $letter) $($ActionItems[$j].Label)"
        if ($isSelected) {
            Write-Host $lineText -ForegroundColor Black -BackgroundColor Gray
        }
        else {
            Write-Host $lineText -ForegroundColor Cyan
        }
    }

    Write-Host ''
    Write-Host ('=' * $width) -ForegroundColor DarkCyan
    if ($ArrowMode) {
        Write-Host ' Up/Down = move   Enter = select/toggle   1-9 = jump   Esc = quit' -ForegroundColor DarkGray
    }
    else {
        Write-Host ' Type a row number to edit/toggle it, or P / R / Q for actions.' -ForegroundColor DarkGray
    }
    Write-Host ('=' * $width) -ForegroundColor DarkCyan
}

# ----------------------------------------------------------------------------
# Editing one item / running actions
# ----------------------------------------------------------------------------

function Invoke-MenuItemAction {
    param($Item, [System.Collections.Specialized.OrderedDictionary]$State)

    switch ($Item.Type) {
        'Text' {
            $State[$Item.Id] = Read-NonEmptyOrKeep -Prompt $Item.Label -Default $State[$Item.Id]
        }
        'List' {
            $State[$Item.Id] = Read-CommaList -Prompt $Item.Label -Current $State[$Item.Id]
        }
        'ExtList' {
            $currentDisplay = if ($null -ne $State[$Item.Id]) { $State[$Item.Id] -join ', ' } else { '(built-in default list)' }
            Write-Host "Current: $currentDisplay"
            $val = Read-Host 'Extensions, e.g. .txt,.log (blank = keep current, DEFAULT = revert to built-in list)'
            if ([string]::IsNullOrWhiteSpace($val)) {
                # keep current, no change
            }
            elseif ($val.Trim().ToUpperInvariant() -eq 'DEFAULT') {
                $State[$Item.Id] = $null
            }
            else {
                $State[$Item.Id] = @($val -split ',' | ForEach-Object { Remove-SurroundingQuotes $_.Trim() } | Where-Object { $_ -ne '' })
            }
        }
        'Choice' {
            $State[$Item.Id] = Read-ChoiceValue -Prompt $Item.Label -Choices $Item.Choices -Default $State[$Item.Id]
        }
        'Int' {
            $State[$Item.Id] = Read-IntValue -Prompt $Item.Label -Default $State[$Item.Id]
        }
        'Double' {
            $State[$Item.Id] = Read-DoubleValue -Prompt $Item.Label -Default $State[$Item.Id]
        }
    }
}

function Build-ParamHashtable {
    param([System.Collections.Specialized.OrderedDictionary]$State)
    $p = [ordered]@{
        SearchPath        = $State.SearchPath
        OutputFolder      = $State.OutputFolder
        Filter            = $State.Filter
        MatchMode         = $State.MatchMode
        ProximityLines    = $State.ProximityLines
        ExcludeScope      = $State.ExcludeScope
        GroupBy           = $State.GroupBy
        MaxFileSizeMB     = $State.MaxFileSizeMB
        MaxEmbedLines     = $State.MaxEmbedLines
        PdfTimeoutSeconds = $State.PdfTimeoutSeconds
        ThrottleLimit     = $State.ThrottleLimit
        MaxRetries        = $State.MaxRetries
        RetryDelayMs      = $State.RetryDelayMs
        FileTimeoutSeconds = $State.FileTimeoutSeconds
    }
    if ($State.OutputName) { $p['OutputName'] = $State.OutputName }
    if ($State.ExcludeFilter -and @($State.ExcludeFilter).Count -gt 0) { $p['ExcludeFilter'] = $State.ExcludeFilter }
    if ($null -ne $State.Extensions) { $p['Extensions'] = $State.Extensions }
    if ($State.ExcludeFolder -and @($State.ExcludeFolder).Count -gt 0) { $p['ExcludeFolder'] = $State.ExcludeFolder }
    if ($State.WholeWord) { $p['WholeWord'] = $true }
    if ($State.UseRegex) { $p['UseRegex'] = $true }
    if ($State.IncludeHidden) { $p['IncludeHidden'] = $true }
    if ($State.OpenReport) { $p['OpenReport'] = $true }
    if ($State.ExportCsv) { $p['ExportCsv'] = $true }
    if ($State.ExportJson) { $p['ExportJson'] = $true }
    if ($State.Parallel) { $p['Parallel'] = $true }
    if ($State.CacheFile) { $p['CacheFile'] = $State.CacheFile }
    if ($State.DryRun) { $p['DryRun'] = $true }
    return $p
}

function Format-CommandPreview {
    param([System.Collections.Specialized.OrderedDictionary]$Params, [string]$ScriptPath)
    $parts = New-Object System.Collections.Generic.List[string]
    [void]$parts.Add("& '$ScriptPath'")
    foreach ($key in $Params.Keys) {
        $val = $Params[$key]
        if ($val -is [bool]) {
            if ($val) { [void]$parts.Add("-$key") }
        }
        elseif ($val -is [array]) {
            $quoted = ($val | ForEach-Object { "'$_'" }) -join ','
            [void]$parts.Add("-$key $quoted")
        }
        else {
            [void]$parts.Add("-$key '$val'")
        }
    }
    return ($parts -join ' ')
}

function Invoke-ActionItem {
    param($ActionId, [System.Collections.Specialized.OrderedDictionary]$State, [string]$MainScriptPath)

    switch ($ActionId) {
        '__PREVIEW__' {
            $paramsPreview = Build-ParamHashtable -State $State
            Write-Host ''
            Write-Host (Format-CommandPreview -Params $paramsPreview -ScriptPath $MainScriptPath) -ForegroundColor Gray
            Write-Host ''
            Read-Host 'Press Enter to continue' | Out-Null
            return $false
        }
        '__RUN__' {
            if (-not $State.SearchPath) {
                Write-Host 'Search folder is required.' -ForegroundColor Red
                Read-Host 'Press Enter to continue' | Out-Null
                return $false
            }
            if (-not $State.OutputFolder) {
                Write-Host 'Output folder is required.' -ForegroundColor Red
                Read-Host 'Press Enter to continue' | Out-Null
                return $false
            }
            if (-not $State.Filter -or @($State.Filter).Count -eq 0) {
                Write-Host 'At least one filter is required.' -ForegroundColor Red
                Read-Host 'Press Enter to continue' | Out-Null
                return $false
            }

            $finalParams = Build-ParamHashtable -State $State
            Write-Host ''
            Write-Host 'Running:' -ForegroundColor Cyan
            Write-Host (Format-CommandPreview -Params $finalParams -ScriptPath $MainScriptPath) -ForegroundColor Gray
            Write-Host ''

            try {
                & $MainScriptPath @finalParams
            }
            catch {
                Write-Host ''
                Write-Host 'The search script reported an error:' -ForegroundColor Red
                Write-Host $_.Exception.Message -ForegroundColor Red
            }

            if (-not $NoSaveSettings) { Save-MenuSettings -State $State }
            Write-Host ''
            Read-Host 'Done. Press Enter to return to the menu' | Out-Null
            return $false
        }
        '__QUIT__' {
            if (-not $NoSaveSettings) { Save-MenuSettings -State $State }
            Write-Host 'Exiting.' -ForegroundColor Yellow
            return $true
        }
    }
    return $false
}

# ----------------------------------------------------------------------------
# Numbered fallback loop (always works - piped input, redirected consoles,
# hosts without raw key support)
# ----------------------------------------------------------------------------

function Invoke-NumberedMenuLoop {
    param([System.Collections.Specialized.OrderedDictionary]$State, [array]$MenuItems, [array]$ActionItems, [string]$MainScriptPath)

    while ($true) {
        Show-MenuScreen -State $State -MenuItems $MenuItems -ActionItems $ActionItems -SelectedIndex -1
        $choice = (Read-Host 'Select an option').Trim()

        if ($choice -match '^[Pp]$') {
            [void](Invoke-ActionItem -ActionId '__PREVIEW__' -State $State -MainScriptPath $MainScriptPath)
            continue
        }
        if ($choice -match '^[Rr]$') {
            [void](Invoke-ActionItem -ActionId '__RUN__' -State $State -MainScriptPath $MainScriptPath)
            continue
        }
        if ($choice -match '^[Qq]$') {
            [void](Invoke-ActionItem -ActionId '__QUIT__' -State $State -MainScriptPath $MainScriptPath)
            return
        }

        $n = 0
        if ([int]::TryParse($choice, [ref]$n) -and $n -ge 1 -and $n -le $MenuItems.Count) {
            $item = $MenuItems[$n - 1]
            if ($item.Type -eq 'Bool') {
                $State[$item.Id] = -not $State[$item.Id]
            }
            else {
                Invoke-MenuItemAction -Item $item -State $State
            }
        }
        else {
            Write-Host "Unrecognized option: $choice" -ForegroundColor Yellow
            Start-Sleep -Milliseconds 700
        }
    }
}

# ----------------------------------------------------------------------------
# Arrow-key loop (used only when the console supports it; falls back
# automatically and immediately if anything about key reading fails)
# ----------------------------------------------------------------------------

function Invoke-ArrowMenuLoop {
    param([System.Collections.Specialized.OrderedDictionary]$State, [array]$MenuItems, [array]$ActionItems, [string]$MainScriptPath)

    $selected = 0
    $total = $MenuItems.Count + $ActionItems.Count

    while ($true) {
        Show-MenuScreen -State $State -MenuItems $MenuItems -ActionItems $ActionItems -SelectedIndex $selected -ArrowMode

        $keyInfo = $null
        try {
            $keyInfo = [Console]::ReadKey($true)
        }
        catch {
            Write-Host ''
            Write-Host 'Arrow-key input is not available in this console session; switching to numbered mode.' -ForegroundColor Yellow
            Start-Sleep -Milliseconds 1200
            Invoke-NumberedMenuLoop -State $State -MenuItems $MenuItems -ActionItems $ActionItems -MainScriptPath $MainScriptPath
            return
        }

        switch ($keyInfo.Key) {
            'UpArrow' { $selected = ($selected - 1 + $total) % $total }
            'DownArrow' { $selected = ($selected + 1) % $total }
            'Home' { $selected = 0 }
            'End' { $selected = $total - 1 }
            'Escape' {
                if (-not $NoSaveSettings) { Save-MenuSettings -State $State }
                return
            }
            'Enter' {
                if ($selected -lt $MenuItems.Count) {
                    $item = $MenuItems[$selected]
                    if ($item.Type -eq 'Bool') {
                        $State[$item.Id] = -not $State[$item.Id]
                    }
                    else {
                        try { Clear-Host } catch { }
                        Write-Host "Editing: $($item.Label)" -ForegroundColor Cyan
                        Write-Host ''
                        Invoke-MenuItemAction -Item $item -State $State
                    }
                }
                else {
                    $actionIds = @('__PREVIEW__', '__RUN__', '__QUIT__')
                    $shouldQuit = Invoke-ActionItem -ActionId $actionIds[$selected - $MenuItems.Count] -State $State -MainScriptPath $MainScriptPath
                    if ($shouldQuit) { return }
                }
            }
            default {
                $ch = $keyInfo.KeyChar
                if ($ch -match '^[1-9]$') {
                    $jumpIndex = [int]"$ch" - 1
                    if ($jumpIndex -lt $total) { $selected = $jumpIndex }
                }
            }
        }
    }
}

# ----------------------------------------------------------------------------
# Entry point
# ----------------------------------------------------------------------------

if (Test-InteractiveConsole) {
    Invoke-ArrowMenuLoop -State $state -MenuItems $menuItems -ActionItems $actionItems -MainScriptPath $MainScriptPath
}
else {
    Invoke-NumberedMenuLoop -State $state -MenuItems $menuItems -ActionItems $actionItems -MainScriptPath $MainScriptPath
}
