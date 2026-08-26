//! Interactive menu mode (`search-cli --interactive` / `-i`).
//!
//! Prompts the user for the same fields `Cli`'s flags cover, then hands
//! back a fully-populated `Cli` for `run()` to execute unchanged - the
//! wizard is purely an alternative way to fill in the same struct, never
//! a second code path for actually running a search (see main.rs's
//! `run()`, which is oblivious to whether its `Cli` came from clap or
//! from here).
//!
//! Any field already supplied on the command line alongside `--interactive`
//! (e.g. `search-cli --interactive /some/folder -f engine`) is kept as-is
//! and not re-prompted for - `--interactive` fills in what's missing,
//! it doesn't discard what's already there.

use std::path::PathBuf;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use crate::{Cli, CliExcludeScope, CliGroupBy, CliMatchMode};

pub(crate) fn gather(defaults: Cli) -> Result<Cli, dialoguer::Error> {
    let theme = ColorfulTheme::default();
    println!("search-cli interactive mode - press Enter to accept a default.\n");

    let search_path: PathBuf = match defaults.search_path {
        Some(p) => p,
        None => {
            let s: String = Input::with_theme(&theme)
                .with_prompt("Folder to search (recursively)")
                .validate_with(|s: &String| -> Result<(), &str> {
                    if s.trim().is_empty() { Err("a folder is required") } else { Ok(()) }
                })
                .interact_text()?;
            PathBuf::from(s)
        }
    };

    let filters: Vec<String> = if !defaults.filters.is_empty() {
        defaults.filters
    } else {
        let mut filters = Vec::new();
        loop {
            let prompt = if filters.is_empty() { "Filter text (required)".to_string() } else { format!("Another filter (#{}, blank to stop)", filters.len() + 1) };
            let f: String = Input::with_theme(&theme).with_prompt(prompt).allow_empty(!filters.is_empty()).interact_text()?;
            if f.trim().is_empty() {
                break;
            }
            filters.push(f);
        }
        filters
    };

    let mode_options = ["Any line matches any filter", "All filters must appear somewhere in the file", "All filters within N lines of each other (proximity)"];
    let mode_idx = Select::with_theme(&theme).with_prompt("Match mode").items(&mode_options).default(0).interact()?;
    let mode = [CliMatchMode::AnyLine, CliMatchMode::AllInFile, CliMatchMode::Proximity][mode_idx];
    let proximity_lines = if mode_idx == 2 {
        Input::with_theme(&theme).with_prompt("Proximity: max lines apart").default(defaults.proximity_lines).interact_text()?
    } else {
        defaults.proximity_lines
    };

    let regex = Confirm::with_theme(&theme).with_prompt("Treat filters as regular expressions?").default(defaults.regex).interact()?;
    let whole_word = if !regex {
        Confirm::with_theme(&theme).with_prompt("Whole-word matching only?").default(defaults.whole_word).interact()?
    } else {
        false
    };

    let advanced = Confirm::with_theme(&theme)
        .with_prompt("Configure advanced options (excludes, extensions, size limit, group-by, export formats, indexing)?")
        .default(false)
        .interact()?;

    let mut exclude_filters = defaults.exclude_filters;
    let mut exclude_scope = defaults.exclude_scope;
    let mut extensions = defaults.extensions;
    let mut exclude_folders = defaults.exclude_folders;
    let mut include_hidden = defaults.include_hidden;
    let mut max_file_size_mb = defaults.max_file_size_mb;
    let mut group_by = defaults.group_by;
    let mut no_parallel = defaults.no_parallel;
    let mut dry_run = defaults.dry_run;
    let mut csv = defaults.csv;
    let mut json = defaults.json;
    let mut jsonl = defaults.jsonl;
    let mut index = defaults.index;
    let output_folder = defaults.output_folder;
    let output_name = defaults.output_name;

    if advanced {
        let exclude_text: String = Input::with_theme(&theme)
            .with_prompt("Exclude filter text, comma-separated (blank for none)")
            .allow_empty(true)
            .default(exclude_filters.join(","))
            .interact_text()?;
        exclude_filters = split_nonempty(&exclude_text);

        if !exclude_filters.is_empty() {
            let scope_idx = Select::with_theme(&theme)
                .with_prompt("Exclude filters apply to")
                .items(&["Just the matching line", "The whole file"])
                .default(0)
                .interact()?;
            exclude_scope = [CliExcludeScope::Line, CliExcludeScope::File][scope_idx];
        }

        let ext_text: String = Input::with_theme(&theme)
            .with_prompt("Extensions to search, comma-separated (blank = default catalog)")
            .allow_empty(true)
            .default(extensions.as_ref().map(|e| e.join(",")).unwrap_or_default())
            .interact_text()?;
        extensions = if ext_text.trim().is_empty() { None } else { Some(split_nonempty(&ext_text)) };

        let exclude_folders_text: String = Input::with_theme(&theme)
            .with_prompt("Folder names to exclude, comma-separated (blank for none)")
            .allow_empty(true)
            .default(exclude_folders.join(","))
            .interact_text()?;
        exclude_folders = split_nonempty(&exclude_folders_text);

        include_hidden = Confirm::with_theme(&theme).with_prompt("Include hidden files/folders?").default(include_hidden).interact()?;
        max_file_size_mb = Input::with_theme(&theme).with_prompt("Max file size (MB)").default(max_file_size_mb).interact_text()?;

        let group_options = ["Created date", "Modified date", "No grouping"];
        let group_idx = Select::with_theme(&theme).with_prompt("Group results by").items(&group_options).default(0).interact()?;
        group_by = [CliGroupBy::Created, CliGroupBy::Modified, CliGroupBy::None][group_idx];

        no_parallel = !Confirm::with_theme(&theme).with_prompt("Use parallel processing?").default(!no_parallel).interact()?;
        dry_run = Confirm::with_theme(&theme).with_prompt("Dry run (list files, read/write nothing)?").default(dry_run).interact()?;

        let export_options = ["CSV", "JSON", "JSON Lines (.jsonl)"];
        let mut export_defaults = vec![csv, json, jsonl];
        let picked = MultiSelect::with_theme(&theme)
            .with_prompt("Export formats in addition to the HTML report (space to toggle)")
            .items(&export_options)
            .defaults(&export_defaults)
            .interact()?;
        export_defaults = vec![false, false, false];
        for i in picked {
            export_defaults[i] = true;
        }
        csv = export_defaults[0];
        json = export_defaults[1];
        jsonl = export_defaults[2];

        index = Confirm::with_theme(&theme).with_prompt("Also build/update the fast-search index for this folder?").default(index).interact()?;
    }

    println!();
    Ok(Cli {
        search_path: Some(search_path),
        filters,
        interactive: false,
        output_folder,
        output_name,
        exclude_filters,
        exclude_scope,
        mode,
        proximity_lines,
        regex,
        whole_word,
        extensions,
        exclude_folders,
        include_hidden,
        max_file_size_mb,
        group_by,
        no_parallel,
        throttle_limit: defaults.throttle_limit,
        heavy_throttle_limit: defaults.heavy_throttle_limit,
        dry_run,
        csv,
        json,
        jsonl,
        index,
        failure_log: defaults.failure_log,
    })
}

fn split_nonempty(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}
