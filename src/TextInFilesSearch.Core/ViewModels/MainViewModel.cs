using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using TextInFilesSearch.Helpers;
using TextInFilesSearch.Models;
using TextInFilesSearch.Services;

namespace TextInFilesSearch.ViewModels;

/// <summary>Lightweight display wrapper around one file's result, for binding to a results list without exposing the whole engine model.</summary>
public sealed class FileResultViewModel
{
    public string FullName { get; init; } = string.Empty;
    public string FileName => Path.GetFileName(FullName);
    public int HitCount { get; init; }
    public DateTime Created { get; init; }
    public DateTime Modified { get; init; }
    public bool LowConfidencePdf { get; init; }
}

/// <summary>
/// The main window's ViewModel: holds every user-configurable setting,
/// exposes Run/Cancel commands, and surfaces live progress (including
/// per-file in-flight status) while a search is running - the direct answer
/// to "a slow PDF looks hung with no insight into what's actually happening".
///
/// Folder browsing and "open the finished report" are injected as delegates
/// rather than called directly, so this whole ViewModel - including the run
/// lifecycle, cancellation, and progress plumbing - can be unit tested
/// without any WinUI/Windows App SDK dependency.
/// </summary>
public sealed class MainViewModel : ObservableObject
{
    private readonly Func<Task<string?>> _browseSearchFolder;
    private readonly Func<Task<string?>> _browseOutputFolder;
    private readonly Action<string> _openReport;

    public MainViewModel(
        Func<Task<string?>>? browseSearchFolder = null,
        Func<Task<string?>>? browseOutputFolder = null,
        Action<string>? openReport = null)
    {
        _browseSearchFolder = browseSearchFolder ?? (() => Task.FromResult<string?>(null));
        _browseOutputFolder = browseOutputFolder ?? (() => Task.FromResult<string?>(null));
        _openReport = openReport ?? (_ => { });

        BrowseSearchFolderCommand = new AsyncRelayCommand(async () =>
        {
            var picked = await _browseSearchFolder();
            if (picked is not null) SearchPath = picked;
        });

        BrowseOutputFolderCommand = new AsyncRelayCommand(async () =>
        {
            var picked = await _browseOutputFolder();
            if (picked is not null) OutputFolder = picked;
        });

        RunSearchCommand = new AsyncRelayCommand(RunSearchAsync, () => !IsRunning && !string.IsNullOrWhiteSpace(SearchPath) && !string.IsNullOrWhiteSpace(OutputFolder) && !string.IsNullOrWhiteSpace(FiltersText));
        CancelCommand = new RelayCommand(() => _cts?.Cancel(), () => IsRunning);
        OpenReportCommand = new RelayCommand(() => { if (LastReportPath is not null) _openReport(LastReportPath); }, () => LastReportPath is not null);
        AddCustomExtensionCommand = new RelayCommand(() => AddCustomExtension(ExtensionFilterText), () => !string.IsNullOrWhiteSpace(ExtensionFilterText));
        ClearSelectedExtensionsCommand = new RelayCommand(() =>
        {
            foreach (var e in ExtensionCatalog) e.IsSelected = false;
        });

        foreach (var category in Models.ExtensionCatalog.Categories)
        {
            foreach (var ext in category.Extensions)
            {
                var option = new ExtensionOption { Extension = ext, Category = category.Category };
                option.PropertyChanged += (_, __) => RefreshSelectedExtensionsSummary();
                ExtensionCatalog.Add(option);
            }
        }
        RefreshFilteredExtensionCatalog();
    }

    // ---------------------------------------------------------------
    // Settings (bound directly from the view's input controls)
    // ---------------------------------------------------------------

    private string _searchPath = string.Empty;
    public string SearchPath { get => _searchPath; set { if (SetProperty(ref _searchPath, value)) RefreshCanRun(); } }

    private string _outputFolder = string.Empty;
    public string OutputFolder { get => _outputFolder; set { if (SetProperty(ref _outputFolder, value)) RefreshCanRun(); } }

    private string? _outputName;
    public string? OutputName { get => _outputName; set => SetProperty(ref _outputName, value); }

    private string _filtersText = string.Empty;
    public string FiltersText { get => _filtersText; set { if (SetProperty(ref _filtersText, value)) RefreshCanRun(); } }

    private string _excludeFiltersText = string.Empty;
    public string ExcludeFiltersText { get => _excludeFiltersText; set => SetProperty(ref _excludeFiltersText, value); }

    private MatchMode _matchMode = MatchMode.AnyLine;
    public MatchMode MatchMode { get => _matchMode; set => SetProperty(ref _matchMode, value); }

    private int _proximityLines = 5;
    public int ProximityLines { get => _proximityLines; set => SetProperty(ref _proximityLines, Math.Max(0, value)); }

    private ExcludeScope _excludeScope = ExcludeScope.Line;
    public ExcludeScope ExcludeScope { get => _excludeScope; set => SetProperty(ref _excludeScope, value); }

    private bool _wholeWord;
    public bool WholeWord { get => _wholeWord; set => SetProperty(ref _wholeWord, value); }

    private bool _useRegex;
    public bool UseRegex { get => _useRegex; set => SetProperty(ref _useRegex, value); }

    private GroupByMode _groupBy = GroupByMode.Created;
    public GroupByMode GroupBy { get => _groupBy; set => SetProperty(ref _groupBy, value); }

    // ---------------------------------------------------------------
    // Extension picker: type-to-filter, then tick one or more. Typing
    // narrows ExtensionCatalog down to FilteredExtensionCatalog; ticking an
    // entry adds/removes it from the effective search extension list built
    // in BuildSettings(). A typed extension that isn't in the built-in
    // catalog can be added as a one-off custom entry via AddCustomExtension.
    // ---------------------------------------------------------------

    public ObservableCollection<ExtensionOption> ExtensionCatalog { get; } = new();
    public ObservableCollection<ExtensionOption> FilteredExtensionCatalog { get; } = new();

    private string _extensionFilterText = string.Empty;
    public string ExtensionFilterText
    {
        get => _extensionFilterText;
        set
        {
            if (SetProperty(ref _extensionFilterText, value))
            {
                RefreshFilteredExtensionCatalog();
                AddCustomExtensionCommand.RaiseCanExecuteChanged();
            }
        }
    }

    private string _selectedExtensionsSummaryText = "Using built-in default extension list.";
    public string SelectedExtensionsSummaryText { get => _selectedExtensionsSummaryText; private set => SetProperty(ref _selectedExtensionsSummaryText, value); }

    private void RefreshFilteredExtensionCatalog()
    {
        FilteredExtensionCatalog.Clear();
        string needle = _extensionFilterText.Trim();
        IEnumerable<ExtensionOption> matches = needle.Length == 0
            ? ExtensionCatalog
            : ExtensionCatalog.Where(e =>
                e.Extension.Contains(needle, StringComparison.OrdinalIgnoreCase) ||
                e.Category.Contains(needle, StringComparison.OrdinalIgnoreCase));

        foreach (var e in matches) FilteredExtensionCatalog.Add(e);
    }

    private void RefreshSelectedExtensionsSummary()
    {
        var selected = ExtensionCatalog.Where(e => e.IsSelected).Select(e => e.Extension).ToList();
        SelectedExtensionsSummaryText = selected.Count == 0
            ? "Using built-in default extension list."
            : $"Searching: {string.Join(", ", selected)}";
    }

    /// <summary>Normalizes user-typed extension text (auto-prepends '.', lowercases) and adds it as a selected custom catalog entry if it isn't already present.</summary>
    public void AddCustomExtension(string rawText)
    {
        string trimmed = rawText.Trim();
        if (trimmed.Length == 0) return;

        string normalized = trimmed.StartsWith('.') ? trimmed : "." + trimmed;
        normalized = normalized.ToLowerInvariant();

        var existing = ExtensionCatalog.FirstOrDefault(e => string.Equals(e.Extension, normalized, StringComparison.OrdinalIgnoreCase));
        if (existing is not null)
        {
            existing.IsSelected = true;
        }
        else
        {
            var option = new ExtensionOption { Extension = normalized, Category = "Custom", IsSelected = true };
            option.PropertyChanged += (_, __) => RefreshSelectedExtensionsSummary();
            ExtensionCatalog.Add(option);
        }

        ExtensionFilterText = string.Empty;
        RefreshSelectedExtensionsSummary();
    }

    private string _excludeFoldersText = string.Empty;
    public string ExcludeFoldersText { get => _excludeFoldersText; set => SetProperty(ref _excludeFoldersText, value); }

    private bool _includeHidden;
    public bool IncludeHidden { get => _includeHidden; set => SetProperty(ref _includeHidden, value); }

    private double _maxFileSizeMB = 50;
    public double MaxFileSizeMB { get => _maxFileSizeMB; set => SetProperty(ref _maxFileSizeMB, Math.Max(0.01, value)); }

    private int _maxEmbedLines = 4000;
    public int MaxEmbedLines { get => _maxEmbedLines; set => SetProperty(ref _maxEmbedLines, Math.Max(1, value)); }

    private int _pdfTimeoutSeconds = 15;
    public int PdfTimeoutSeconds { get => _pdfTimeoutSeconds; set => SetProperty(ref _pdfTimeoutSeconds, Math.Max(1, value)); }

    private bool _openReportWhenDone;
    public bool OpenReportWhenDone { get => _openReportWhenDone; set => SetProperty(ref _openReportWhenDone, value); }

    private bool _exportCsv;
    public bool ExportCsv { get => _exportCsv; set => SetProperty(ref _exportCsv, value); }

    private bool _exportJson;
    public bool ExportJson { get => _exportJson; set => SetProperty(ref _exportJson, value); }

    private bool _parallel;
    public bool Parallel { get => _parallel; set => SetProperty(ref _parallel, value); }

    private int _throttleLimit = 5;
    public int ThrottleLimit { get => _throttleLimit; set => SetProperty(ref _throttleLimit, Math.Max(1, value)); }

    private string? _cacheFilePath;
    public string? CacheFilePath { get => _cacheFilePath; set => SetProperty(ref _cacheFilePath, value); }

    private bool _dryRun;
    public bool DryRun { get => _dryRun; set => SetProperty(ref _dryRun, value); }

    private int _maxRetries = 3;
    public int MaxRetries { get => _maxRetries; set => SetProperty(ref _maxRetries, Math.Max(0, value)); }

    private int _retryDelayMs = 250;
    public int RetryDelayMs { get => _retryDelayMs; set => SetProperty(ref _retryDelayMs, Math.Max(0, value)); }

    private int _fileTimeoutSeconds = 30;
    public int FileTimeoutSeconds { get => _fileTimeoutSeconds; set => SetProperty(ref _fileTimeoutSeconds, Math.Max(1, value)); }

    // ---------------------------------------------------------------
    // Live run state
    // ---------------------------------------------------------------

    private bool _isRunning;
    public bool IsRunning
    {
        get => _isRunning;
        private set
        {
            if (SetProperty(ref _isRunning, value))
            {
                RefreshCanRun();
                (CancelCommand as RelayCommand)?.RaiseCanExecuteChanged();
            }
        }
    }

    private double _progressPercent;
    public double ProgressPercent { get => _progressPercent; private set => SetProperty(ref _progressPercent, value); }

    private string _statusText = "Ready.";
    public string StatusText { get => _statusText; private set => SetProperty(ref _statusText, value); }

    public ObservableCollection<InFlightFileStatus> InFlightFiles { get; } = new();

    public ObservableCollection<FileResultViewModel> Results { get; } = new();

    private string _resultsSummaryText = string.Empty;
    public string ResultsSummaryText { get => _resultsSummaryText; private set => SetProperty(ref _resultsSummaryText, value); }

    private bool _hasResults;
    public bool HasResults { get => _hasResults; private set => SetProperty(ref _hasResults, value); }

    private string? _lastReportPath;
    public string? LastReportPath
    {
        get => _lastReportPath;
        private set { if (SetProperty(ref _lastReportPath, value)) (OpenReportCommand as RelayCommand)?.RaiseCanExecuteChanged(); }
    }

    private CancellationTokenSource? _cts;

    /// <summary>
    /// Guards every mutation of Results/InFlightFiles from OnProgress against
    /// concurrent execution. Progress&lt;T&gt; marshals its callback via
    /// whatever SynchronizationContext was captured when it was constructed:
    /// the UI dispatcher in the real app, which serializes every call onto
    /// one thread - but genuinely concurrently on the thread pool when there
    /// is none (e.g. a console test harness), and a report can still be
    /// in flight on a background thread even after RunAsync's own await has
    /// completed. Without this lock, two overlapping calls (or a still-
    /// draining one racing the final reconciliation below) can both pass a
    /// "not already added" check for the same file and double-add it into
    /// the non-thread-safe ObservableCollection.
    /// </summary>
    private readonly object _progressLock = new();

    /// <summary>
    /// Set once RunSearchAsync has finished (successfully, cancelled, or
    /// errored) so a still-draining, stale Progress&lt;T&gt; callback from
    /// before that point can't land afterward and overwrite the final
    /// StatusText/ProgressPercent with a mid-run value.
    /// </summary>
    private volatile bool _runConcluded = true;

    // ---------------------------------------------------------------
    // Commands
    // ---------------------------------------------------------------

    public AsyncRelayCommand BrowseSearchFolderCommand { get; }
    public AsyncRelayCommand BrowseOutputFolderCommand { get; }
    public AsyncRelayCommand RunSearchCommand { get; }
    public RelayCommand CancelCommand { get; }
    public RelayCommand OpenReportCommand { get; }
    public RelayCommand AddCustomExtensionCommand { get; }
    public RelayCommand ClearSelectedExtensionsCommand { get; }

    private void RefreshCanRun() => RunSearchCommand.RaiseCanExecuteChanged();

    // ---------------------------------------------------------------
    // Settings <-> free-text field parsing
    // ---------------------------------------------------------------

    private static List<string> ParseList(string text) =>
        text.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).ToList();

    /// <summary>Null means "use the built-in default list" (same convention SearchSettings.Extensions already used) - so ticking nothing in the picker behaves exactly like the old blank free-text box did.</summary>
    private List<string>? BuildSelectedExtensions()
    {
        var selected = ExtensionCatalog.Where(e => e.IsSelected).Select(e => e.Extension).ToList();
        return selected.Count > 0 ? selected : null;
    }

    /// <summary>Replaces every character Windows disallows in a file name with '_' - a stray ':'/'*'/'?' etc. in a user-typed report name previously threw at save time instead of being caught proactively.</summary>
    private static string SanitizeFileName(string name)
    {
        var invalid = Path.GetInvalidFileNameChars();
        var sb = new StringBuilder(name.Length);
        foreach (char c in name) sb.Append(invalid.Contains(c) ? '_' : c);
        return sb.ToString();
    }

    public SearchSettings BuildSettings() => new()
    {
        SearchPath = SearchPath.Trim(),
        OutputFolder = OutputFolder.Trim(),
        OutputName = string.IsNullOrWhiteSpace(OutputName) ? null : SanitizeFileName(OutputName),
        Filters = ParseList(FiltersText),
        ExcludeFilters = ParseList(ExcludeFiltersText),
        MatchMode = MatchMode,
        ProximityLines = ProximityLines,
        ExcludeScope = ExcludeScope,
        WholeWord = WholeWord,
        UseRegex = UseRegex,
        GroupBy = GroupBy,
        Extensions = BuildSelectedExtensions(),
        ExcludeFolders = ParseList(ExcludeFoldersText),
        IncludeHidden = IncludeHidden,
        MaxFileSizeMB = MaxFileSizeMB,
        MaxEmbedLines = MaxEmbedLines,
        PdfTimeoutSeconds = PdfTimeoutSeconds,
        ExportCsv = ExportCsv,
        ExportJson = ExportJson,
        OpenReportWhenDone = OpenReportWhenDone,
        Parallel = Parallel,
        ThrottleLimit = ThrottleLimit,
        CacheFilePath = string.IsNullOrWhiteSpace(CacheFilePath) ? null : CacheFilePath,
        DryRun = DryRun,
        MaxRetries = MaxRetries,
        RetryDelayMs = RetryDelayMs,
        FileTimeoutSeconds = FileTimeoutSeconds
    };

    // ---------------------------------------------------------------
    // Run lifecycle
    // ---------------------------------------------------------------

    public async Task RunSearchAsync()
    {
        var settings = BuildSettings();

        Results.Clear();
        InFlightFiles.Clear();
        HasResults = false;
        ResultsSummaryText = string.Empty;
        LastReportPath = null;
        ProgressPercent = 0;
        StatusText = "Starting...";

        _cts = new CancellationTokenSource();
        IsRunning = true;
        _runConcluded = false;

        var progress = new Progress<SearchProgressReport>(OnProgress);

        try
        {
            var orchestrator = new SearchOrchestrator();
            var runResult = await orchestrator.RunAsync(settings, progress, _cts.Token);

            if (runResult.WasDryRun)
            {
                int count = runResult.DryRunCandidates?.Count ?? 0;
                StatusText = $"Dry run: {count} file(s) would be searched. Nothing was read or written.";
                ResultsSummaryText = StatusText;
                return;
            }

            // Results is normally already populated live by OnProgress as
            // each file streams in - a UX nicety, not a completeness or
            // correctness guarantee: Progress<T> marshals its callback via
            // whatever SynchronizationContext was captured at construction
            // (the UI dispatcher in the real app, which serializes every
            // call onto one thread; the thread pool - genuinely
            // concurrently - when there is none, e.g. a console test
            // harness). Concurrent delivery can race two OnProgress calls
            // past each other's "not already added" check for the same
            // file. Rebuilding from scratch here (rather than merging with
            // whatever the live phase left behind) means Results is always
            // exactly correct regardless of that timing, with no risk of a
            // race-induced duplicate surviving into the final state.
            lock (_progressLock)
            {
                Results.Clear();
                foreach (var r in runResult.FileResults.Where(r => r.Status == FileSearchStatus.Hit))
                {
                    Results.Add(new FileResultViewModel
                    {
                        FullName = r.FullName,
                        HitCount = r.Hits.Count,
                        Created = r.Created,
                        Modified = r.Modified,
                        LowConfidencePdf = r.LowConfidencePdf
                    });
                }
                HasResults = Results.Count > 0;
            }

            int totalHits = runResult.FileResults.Where(r => r.Status == FileSearchStatus.Hit).Sum(r => r.Hits.Count);
            ResultsSummaryText = $"Searched {runResult.Summary.FilesSearched} file(s). " +
                $"{Results.Count} file(s) with hits, {totalHits} total hits. " +
                $"Skipped: {runResult.Summary.SkippedTooLarge} too large, {runResult.Summary.SkippedBinary} binary, " +
                $"{runResult.Summary.SkippedReadError} unreadable, {runResult.Summary.SkippedUnexpectedError} unexpected errors." +
                (runResult.Summary.EnumerationErrors > 0 ? $" {runResult.Summary.EnumerationErrors} folder(s)/file(s) couldn't be listed (permissions or a broken link)." : string.Empty);

            if (!settings.DryRun)
            {
                string html = ReportExportService.BuildHtmlReport(settings, runResult);
                string outputName = string.IsNullOrWhiteSpace(settings.OutputName)
                    ? $"SearchResults_{DateTime.Now:yyyyMMdd_HHmmss}.html"
                    : (settings.OutputName!.EndsWith(".html", StringComparison.OrdinalIgnoreCase) ? settings.OutputName! : settings.OutputName + ".html");

                Directory.CreateDirectory(settings.OutputFolder);
                string reportPath = Path.Combine(settings.OutputFolder, outputName);
                await File.WriteAllTextAsync(reportPath, html);
                LastReportPath = reportPath;

                if (settings.ExportCsv || settings.ExportJson)
                {
                    var rows = ReportExportService.BuildExportRows(runResult);
                    if (settings.ExportCsv) ReportExportService.WriteCsv(Path.ChangeExtension(reportPath, ".csv"), rows);
                    if (settings.ExportJson) ReportExportService.WriteJson(Path.ChangeExtension(reportPath, ".json"), rows);
                }

                if (settings.OpenReportWhenDone) _openReport(reportPath);
            }

            StatusText = "Done.";
        }
        catch (OperationCanceledException)
        {
            StatusText = "Cancelled.";
        }
        catch (Exception ex)
        {
            StatusText = $"Error: {ex.Message}";
        }
        finally
        {
            IsRunning = false;
            InFlightFiles.Clear();
            _cts?.Dispose();
            _cts = null;
            _runConcluded = true;
        }
    }

    private void OnProgress(SearchProgressReport report)
    {
        // A stale callback still draining after the run has already
        // concluded (see _runConcluded's doc comment) must not clobber the
        // final StatusText/ProgressPercent with a mid-run value.
        if (_runConcluded) return;

        if (report.IsEnumerating)
        {
            // A large or slow (network-share) tree can take a real while just
            // to walk before any file processing even starts - without this,
            // that phase looked exactly like the UI being hung.
            StatusText = report.EnumeratedFileCount > 0
                ? $"Scanning folders... {report.EnumeratedFileCount} file(s) found so far"
                : "Scanning folders...";
            return;
        }

        if (report.TotalFiles > 0)
        {
            ProgressPercent = 100.0 * report.FilesCompleted / report.TotalFiles;
            StatusText = $"{report.FilesCompleted} of {report.TotalFiles} file(s) - {report.HitsSoFar} hit(s) so far";
        }

        lock (_progressLock)
        {
            InFlightFiles.Clear();
            foreach (var f in report.InFlightFiles) InFlightFiles.Add(f);

            if (report.LastCompletedResult is { Status: FileSearchStatus.Hit } r &&
                !Results.Any(existing => string.Equals(existing.FullName, r.FullName, StringComparison.OrdinalIgnoreCase)))
            {
                Results.Add(new FileResultViewModel
                {
                    FullName = r.FullName,
                    HitCount = r.Hits.Count,
                    Created = r.Created,
                    Modified = r.Modified,
                    LowConfidencePdf = r.LowConfidencePdf
                });
                HasResults = true;
            }
        }
    }
}
