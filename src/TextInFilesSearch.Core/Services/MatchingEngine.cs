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
            foreach (var f in settings.Filters)
                state.CompiledFilterRegex[f] = new Regex(f, RegexOptions.IgnoreCase | RegexOptions.Compiled, TimeSpan.FromSeconds(2));
            foreach (var f in settings.ExcludeFilters)
                state.CompiledExcludeRegex[f] = new Regex(f, RegexOptions.IgnoreCase | RegexOptions.Compiled, TimeSpan.FromSeconds(2));
        }
        else if (settings.WholeWord)
        {
            foreach (var f in settings.Filters)
                state.WholeWordFilterRegex[f] = new Regex(@"\b" + Regex.Escape(f) + @"\b", RegexOptions.IgnoreCase | RegexOptions.Compiled);
            foreach (var f in settings.ExcludeFilters)
                state.WholeWordExcludeRegex[f] = new Regex(@"\b" + Regex.Escape(f) + @"\b", RegexOptions.IgnoreCase | RegexOptions.Compiled);
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
            var parts = filters.Select(f => useRegex ? $"(?:{f})" : (wholeWord ? @"\b" + Regex.Escape(f) + @"\b" : Regex.Escape(f)));
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

        var fileMatchedFilters = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

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
                foreach (var f in settings.Filters)
                {
                    if (IsHit(line, f, settings, state.CompiledFilterRegex, state.WholeWordFilterRegex))
                    {
                        matchedFilters.Add(f);
                        fileMatchedFilters.Add(f);
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
            foreach (var f in settings.Filters)
            {
                if (!fileMatchedFilters.Contains(f)) { passesMode = false; break; }
            }
        }

        if (passesMode && settings.MatchMode == MatchMode.Proximity)
        {
            var filterLineLists = new Dictionary<string, List<int>>(StringComparer.OrdinalIgnoreCase);
            foreach (var h in hits)
            {
                foreach (var mf in h.MatchedFilters)
                {
                    if (!filterLineLists.TryGetValue(mf, out var list))
                    {
                        list = new List<int>();
                        filterLineLists[mf] = list;
                    }
                    list.Add(h.LineNumber);
                }
            }
            foreach (var key in filterLineLists.Keys.ToList())
            {
                filterLineLists[key] = filterLineLists[key].Distinct().OrderBy(x => x).ToList();
            }

            int minRange = GetMinLineRangeAcrossFilters(filterLineLists, settings.Filters);
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
    /// Given each filter's sorted, distinct hit-line-numbers within one file,
    /// returns the smallest line span covering at least one line per filter -
    /// the classic "smallest range covering one element from each list"
    /// problem, solved by always advancing whichever list sits at the current
    /// minimum. Assumes every filter has at least one entry (callers only call
    /// this after confirming that via the AllInFile-style gate above).
    /// </summary>
    public static int GetMinLineRangeAcrossFilters(Dictionary<string, List<int>> filterLineLists, IReadOnlyList<string> filters)
    {
        var lists = filters.Select(f => filterLineLists[f]).ToList();
        int k = lists.Count;
        if (k == 0) return int.MaxValue;

        var ptr = new int[k];
        int bestRange = int.MaxValue;

        while (true)
        {
            var vals = new int[k];
            bool exhausted = false;
            for (int i = 0; i < k; i++)
            {
                if (ptr[i] >= lists[i].Count) { exhausted = true; break; }
                vals[i] = lists[i][ptr[i]];
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
