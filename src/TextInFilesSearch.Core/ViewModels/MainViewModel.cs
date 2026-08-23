using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.IO;
using System.Linq;
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
    public int ProximityLines { get => _proximityLines; set => SetProperty(ref _proximityLines, value); }

    private ExcludeScope _excludeScope = ExcludeScope.Line;
    public ExcludeScope ExcludeScope { get => _excludeScope; set => SetProperty(ref _excludeScope, value); }

    private bool _wholeWord;
    public bool WholeWord { get => _wholeWord; set => SetProperty(ref _wholeWord, value); }

    private bool _useRegex;
    public bool UseRegex { get => _useRegex; set => SetProperty(ref _useRegex, value); }

    private GroupByMode _groupBy = GroupByMode.Created;
    public GroupByMode GroupBy { get => _groupBy; set => SetProperty(ref _groupBy, value); }

    private string? _extensionsText;
    public string? ExtensionsText { get => _extensionsText; set => SetProperty(ref _extensionsText, value); }

    private string _excludeFoldersText = string.Empty;
    public string ExcludeFoldersText { get => _excludeFoldersText; set => SetProperty(ref _excludeFoldersText, value); }

    private bool _includeHidden;
    public bool IncludeHidden { get => _includeHidden; set => SetProperty(ref _includeHidden, value); }

    private double _maxFileSizeMB = 50;
    public double MaxFileSizeMB { get => _maxFileSizeMB; set => SetProperty(ref _maxFileSizeMB, value); }

    private int _maxEmbedLines = 4000;
    public int MaxEmbedLines { get => _maxEmbedLines; set => SetProperty(ref _maxEmbedLines, value); }

    private int _pdfTimeoutSeconds = 15;
    public int PdfTimeoutSeconds { get => _pdfTimeoutSeconds; set => SetProperty(ref _pdfTimeoutSeconds, value); }

    private bool _openReportWhenDone;
    public bool OpenReportWhenDone { get => _openReportWhenDone; set => SetProperty(ref _openReportWhenDone, value); }

    private bool _exportCsv;
    public bool ExportCsv { get => _exportCsv; set => SetProperty(ref _exportCsv, value); }

    private bool _exportJson;
    public bool ExportJson { get => _exportJson; set => SetProperty(ref _exportJson, value); }

    private bool _parallel;
    public bool Parallel { get => _parallel; set => SetProperty(ref _parallel, value); }

    private int _throttleLimit = 5;
    public int ThrottleLimit { get => _throttleLimit; set => SetProperty(ref _throttleLimit, value); }

    private string? _cacheFilePath;
    public string? CacheFilePath { get => _cacheFilePath; set => SetProperty(ref _cacheFilePath, value); }

    private bool _dryRun;
    public bool DryRun { get => _dryRun; set => SetProperty(ref _dryRun, value); }

    private int _maxRetries = 3;
    public int MaxRetries { get => _maxRetries; set => SetProperty(ref _maxRetries, value); }

    private int _retryDelayMs = 250;
    public int RetryDelayMs { get => _retryDelayMs; set => SetProperty(ref _retryDelayMs, value); }

    private int _fileTimeoutSeconds = 30;
    public int FileTimeoutSeconds { get => _fileTimeoutSeconds; set => SetProperty(ref _fileTimeoutSeconds, value); }

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

    // ---------------------------------------------------------------
    // Commands
    // ---------------------------------------------------------------

    public AsyncRelayCommand BrowseSearchFolderCommand { get; }
    public AsyncRelayCommand BrowseOutputFolderCommand { get; }
    public AsyncRelayCommand RunSearchCommand { get; }
    public RelayCommand CancelCommand { get; }
    public RelayCommand OpenReportCommand { get; }

    private void RefreshCanRun() => RunSearchCommand.RaiseCanExecuteChanged();

    // ---------------------------------------------------------------
    // Settings <-> free-text field parsing
    // ---------------------------------------------------------------

    private static List<string> ParseList(string text) =>
        text.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).ToList();

    public SearchSettings BuildSettings() => new()
    {
        SearchPath = SearchPath.Trim(),
        OutputFolder = OutputFolder.Trim(),
        OutputName = string.IsNullOrWhiteSpace(OutputName) ? null : OutputName,
        Filters = ParseList(FiltersText),
        ExcludeFilters = ParseList(ExcludeFiltersText),
        MatchMode = MatchMode,
        ProximityLines = ProximityLines,
        ExcludeScope = ExcludeScope,
        WholeWord = WholeWord,
        UseRegex = UseRegex,
        GroupBy = GroupBy,
        Extensions = string.IsNullOrWhiteSpace(ExtensionsText) ? null : ParseList(ExtensionsText),
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

            int totalHits = runResult.FileResults.Where(r => r.Status == FileSearchStatus.Hit).Sum(r => r.Hits.Count);
            ResultsSummaryText = $"Searched {runResult.Summary.FilesSearched} file(s). " +
                $"{Results.Count} file(s) with hits, {totalHits} total hits. " +
                $"Skipped: {runResult.Summary.SkippedTooLarge} too large, {runResult.Summary.SkippedBinary} binary, " +
                $"{runResult.Summary.SkippedReadError} unreadable, {runResult.Summary.SkippedUnexpectedError} unexpected errors.";

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
        }
    }

    private void OnProgress(SearchProgressReport report)
    {
        if (report.TotalFiles > 0)
        {
            ProgressPercent = 100.0 * report.FilesCompleted / report.TotalFiles;
            StatusText = $"{report.FilesCompleted} of {report.TotalFiles} file(s) - {report.HitsSoFar} hit(s) so far";
        }

        InFlightFiles.Clear();
        foreach (var f in report.InFlightFiles) InFlightFiles.Add(f);
    }
}
