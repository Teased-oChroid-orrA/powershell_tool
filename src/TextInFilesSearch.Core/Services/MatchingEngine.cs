using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.RegularExpressions;
using TextInFilesSearch.Models;

namespace TextInFilesSearch.Services;

/// <summary>
/// Precompiled regex state built once per search run (not per file, not per
/// line) and reused across every file - the same optimization as the
/// PowerShell version's compiled-regex cache and combined pre-check pattern.
/// </summary>
/// <summary>
/// Thrown when one or more regex-mode filters fail to compile, identifying
/// exactly which filter(s) and why - previously an invalid filter threw a
/// bare ArgumentException from deep inside Regex construction, caught only
/// generically by the ViewModel as "Error: parsing pattern..." with no
/// indication of which of possibly many filters was the problem.
/// </summary>
public sealed class InvalidFilterRegexException : Exception
{
    public IReadOnlyList<(string Filter, string Error)> InvalidFilters { get; }

    public InvalidFilterRegexException(IReadOnlyList<(string Filter, string Error)> invalidFilters)
        : base(BuildMessage(invalidFilters))
    {
        InvalidFilters = invalidFilters;
    }

    private static string BuildMessage(IReadOnlyList<(string Filter, string Error)> invalidFilters) =>
        "Invalid regex filter(s): " + string.Join("; ", invalidFilters.Select(f => $"\"{f.Filter}\" ({f.Error})"));
}

public sealed class CompiledMatchState
{
    public required IReadOnlyList<string> Filters { get; init; }
    public required IReadOnlyList<string> ExcludeFilters { get; init; }
    public bool UseRegex { get; init; }
    public bool WholeWord { get; init; }

    public Dictionary<string, Regex> CompiledFilterRegex { get; } = new();
    public Dictionary<string, Regex> CompiledExcludeRegex { get; } = new();
    public Dictionary<string, Regex> WholeWordFilterRegex { get; } = new();
    public Dictionary<string, Regex> WholeWordExcludeRegex { get; } = new();

    /// <summary>One alternation of every filter, for a cheap single check per line before the slower per-filter loop.</summary>
    public Regex? CombinedFilterRegex { get; private set; }
    public Regex? CombinedExcludeRegex { get; private set; }

    public static CompiledMatchState Build(SearchSettings settings)
    {
        var state = new CompiledMatchState
        {
            Filters = settings.Filters,
            ExcludeFilters = settings.ExcludeFilters,
            UseRegex = settings.UseRegex,
            WholeWord = settings.WholeWord
        };

        if (settings.UseRegex)
        {
            var invalid = new List<(string Filter, string Error)>();

            foreach (var f in settings.Filters)
            {
                try { state.CompiledFilterRegex[f] = new Regex(f, RegexOptions.IgnoreCase | RegexOptions.Compiled, TimeSpan.FromSeconds(2)); }
                catch (Exception ex) { invalid.Add((f, ex.Message)); }
            }
            foreach (var f in settings.ExcludeFilters)
            {
                try { state.CompiledExcludeRegex[f] = new Regex(f, RegexOptions.IgnoreCase | RegexOptions.Compiled, TimeSpan.FromSeconds(2)); }
                catch (Exception ex) { invalid.Add((f, ex.Message)); }
            }

            if (invalid.Count > 0) throw new InvalidFilterRegexException(invalid);
        }
        else if (settings.WholeWord)
        {
            foreach (var f in settings.Filters)
                state.WholeWordFilterRegex[f] = new Regex(WholeWordHelper.BuildPattern(f), RegexOptions.IgnoreCase | RegexOptions.Compiled);
            foreach (var f in settings.ExcludeFilters)
                state.WholeWordExcludeRegex[f] = new Regex(WholeWordHelper.BuildPattern(f), RegexOptions.IgnoreCase | RegexOptions.Compiled);
        }

        state.CombinedFilterRegex = BuildCombined(settings.Filters, settings.UseRegex, settings.WholeWord);
        state.CombinedExcludeRegex = settings.ExcludeFilters.Count > 0
            ? BuildCombined(settings.ExcludeFilters, settings.UseRegex, settings.WholeWord)
            : null;

        return state;
    }

    private static Regex? BuildCombined(IReadOnlyList<string> filters, bool useRegex, bool wholeWord)
    {
        if (filters.Count == 0) return null;
        try
        {
            var parts = filters.Select(f => useRegex ? $"(?:{f})" : (wholeWord ? WholeWordHelper.BuildPattern(f) : Regex.Escape(f)));
            string pattern = "(?:" + string.Join("|", parts) + ")";
            return new Regex(pattern, RegexOptions.IgnoreCase | RegexOptions.Compiled, TimeSpan.FromSeconds(2));
        }
        catch
        {
            // An invalid combined pattern (e.g. a broken user regex) just means
            // callers fall back to checking every filter on every line - slower,
            // never wrong.
            return null;
        }
    }
}

/// <summary>
/// Builds the "whole word" match pattern used everywhere whole-word mode is
/// needed (MatchingEngine and ReportExportService's highlighter alike - one
/// implementation instead of two that could drift). Uses lookaround against
/// letter/digit/underscore rather than \b: \b only asserts a transition
/// between a \w and non-\w character, so a filter whose own first or last
/// character is itself non-word (e.g. "C#") can fail to match even when
/// standing alone between spaces, because neither side of that boundary is a
/// \w-to-\W transition. Asserting "not adjacent to a letter/digit/underscore"
/// on both sides instead gives the intuitive "isolated token" behavior
/// regardless of what character the filter starts or ends with.
/// </summary>
public static class WholeWordHelper
{
    public static string BuildPattern(string filter) =>
        @"(?<![\p{L}\p{N}_])" + Regex.Escape(filter) + @"(?![\p{L}\p{N}_])";
}

public static class MatchingEngine
{
    /// <summary>
    /// Scans every line of an already-extracted file for filter/exclude
    /// matches, then applies the AllInFile/Proximity gating rules. This is a
    /// direct port of Invoke-SingleFileSearch's line-matching loop.
    ///
    /// passesMode is reported explicitly (rather than inferred from an empty
    /// hits list) so the caller can correctly distinguish "no per-line matches
    /// at all" (NoHit) from "matches existed but the file failed the
    /// AllInFile/Proximity gate" (ModeExcluded) - collapsing those two cases
    /// was a real bug caught during review.
    /// </summary>
    public static void ApplyLineMatching(
        string[] lines,
        SearchSettings settings,
        CompiledMatchState state,
        out List<LineHit> hits,
        out bool excludedByFile,
        out bool passesMode,
        out int? proximityMinRange)
    {
        hits = new List<LineHit>();
        excludedByFile = false;
        passesMode = true;
        proximityMinRange = null;

        // Indexed by filter *slot* (position in settings.Filters), not by
        // filter text - two filters differing only by case (or literal
        // duplicates) are distinct slots. Keying this by string previously
        // let case-variant duplicate filters silently collapse into one
        // entry, skewing the proximity range calculation.
        int filterCount = settings.Filters.Count;
        var perFilterHitLines = new List<int>[filterCount];
        for (int fi = 0; fi < filterCount; fi++) perFilterHitLines[fi] = new List<int>();

        for (int i = 0; i < lines.Length; i++)
        {
            string? line = lines[i];
            if (line is null) continue;

            if (settings.ExcludeFilters.Count > 0)
            {
                bool excludeCandidate = state.CombinedExcludeRegex?.IsMatch(line) ?? true;
                if (excludeCandidate)
                {
                    bool isExcludedLine = false;
                    foreach (var xf in settings.ExcludeFilters)
                    {
                        if (IsHit(line, xf, settings, state.CompiledExcludeRegex, state.WholeWordExcludeRegex))
                        {
                            isExcludedLine = true;
                            break;
                        }
                    }
                    if (isExcludedLine)
                    {
                        if (settings.ExcludeScope == ExcludeScope.File) excludedByFile = true;
                        continue;
                    }
                }
            }

            var matchedFilters = new List<string>();
            bool candidateLine = state.CombinedFilterRegex?.IsMatch(line) ?? true;

            if (candidateLine)
            {
                for (int fi = 0; fi < filterCount; fi++)
                {
                    string f = settings.Filters[fi];
                    if (IsHit(line, f, settings, state.CompiledFilterRegex, state.WholeWordFilterRegex))
                    {
                        matchedFilters.Add(f);
                        perFilterHitLines[fi].Add(i + 1);
                    }
                }
            }

            if (matchedFilters.Count > 0)
            {
                hits.Add(new LineHit
                {
                    LineNumber = i + 1,
                    Before = i > 0 ? lines[i - 1] : null,
                    MatchLine = line,
                    After = i < lines.Length - 1 ? lines[i + 1] : null,
                    MatchedFilters = matchedFilters
                });
            }
        }

        if (excludedByFile) return;

        if (settings.MatchMode is MatchMode.AllInFile or MatchMode.Proximity)
        {
            for (int fi = 0; fi < filterCount; fi++)
            {
                if (perFilterHitLines[fi].Count == 0) { passesMode = false; break; }
            }
        }

        if (passesMode && settings.MatchMode == MatchMode.Proximity)
        {
            // Each per-filter list was appended in increasing line order by
            // construction, so it's already sorted with no duplicates.
            int minRange = GetMinLineRangeAcrossFilters(perFilterHitLines);
            proximityMinRange = minRange;
            if (minRange > settings.ProximityLines) passesMode = false;
        }
    }

    private static bool IsHit(
        string line,
        string filter,
        SearchSettings settings,
        IReadOnlyDictionary<string, Regex> compiledRegex,
        IReadOnlyDictionary<string, Regex> wholeWordRegex)
    {
        try
        {
            if (settings.UseRegex)
                return compiledRegex.TryGetValue(filter, out var rx) && rx.IsMatch(line);
            if (settings.WholeWord)
                return wholeWordRegex.TryGetValue(filter, out var rx2) && rx2.IsMatch(line);
            return line.IndexOf(filter, StringComparison.OrdinalIgnoreCase) >= 0;
        }
        catch (RegexMatchTimeoutException)
        {
            return false;
        }
    }

    /// <summary>
    /// Given each filter slot's sorted hit-line-numbers within one file,
    /// returns the smallest line span covering at least one line per filter -
    /// the classic "smallest range covering one element from each list"
    /// problem, solved by always advancing whichever list sits at the current
    /// minimum. Assumes every filter has at least one entry (callers only call
    /// this after confirming that via the AllInFile-style gate above).
    /// </summary>
    public static int GetMinLineRangeAcrossFilters(IReadOnlyList<List<int>> filterLineLists)
    {
        int k = filterLineLists.Count;
        if (k == 0) return int.MaxValue;

        var ptr = new int[k];
        int bestRange = int.MaxValue;

        while (true)
        {
            var vals = new int[k];
            bool exhausted = false;
            for (int i = 0; i < k; i++)
            {
                if (ptr[i] >= filterLineLists[i].Count) { exhausted = true; break; }
                vals[i] = filterLineLists[i][ptr[i]];
            }
            if (exhausted) break;

            int minVal = vals[0], maxVal = vals[0], minIdx = 0;
            for (int i = 1; i < k; i++)
            {
                if (vals[i] < minVal) { minVal = vals[i]; minIdx = i; }
                if (vals[i] > maxVal) maxVal = vals[i];
            }

            int range = maxVal - minVal;
            if (range < bestRange) bestRange = range;

            ptr[minIdx]++;
        }

        return bestRange;
    }
}
