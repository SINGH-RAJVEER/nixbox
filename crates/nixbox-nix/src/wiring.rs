//! Text edits that wire a GitHub flake into the user's root `flake.nix` and
//! pass the resulting `inputs` set through to the modules that consume it.
//!
//! These edits follow the conventions a hand-maintained configuration already
//! uses: inputs get short names (`zen-browser`, not `"owner/zen-browser-flake"`),
//! follow the root `nixpkgs` when there is one, and are reused rather than
//! duplicated when the user has declared them already.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Adds `github:<repo>` to the root flake's inputs, or finds the input that
/// already points at it, and returns the attribute name to reference it by
/// under `inputs.` (quoted when it is not a plain identifier).
pub fn ensure_flake_input(flake_file: &Path, repo: &str) -> Result<String> {
    let source = fs::read_to_string(flake_file)
        .with_context(|| format!("reading {}", flake_file.display()))?;
    if !outputs_bind_inputs(&source) {
        bail!(
            "{} must bind `inputs` in its outputs function (`outputs = inputs@{{ ... }}:` or \
             `outputs = {{ ... }}@inputs:`) before nixbox can install flake outputs",
            flake_file.display()
        );
    }
    let inputs = RootInputs::parse(&source)
        .with_context(|| format!("could not find the inputs of {}", flake_file.display()))?;
    if let Some(name) = inputs.name_for_repo(repo) {
        return Ok(name.to_string());
    }

    let name = inputs.unused_name(repo);
    let follows = inputs.entries.iter().any(|entry| entry.name == "nixpkgs");
    let updated = inputs.insert(&source, &name, repo, follows);
    fs::write(flake_file, updated).with_context(|| format!("writing {}", flake_file.display()))?;
    Ok(name)
}

/// Drops every binding of the root input `name`, and reports whether there
/// was one.
pub fn remove_flake_input(flake_file: &Path, name: &str) -> Result<bool> {
    if !flake_file.exists() {
        return Ok(false);
    }
    let source = fs::read_to_string(flake_file)
        .with_context(|| format!("reading {}", flake_file.display()))?;
    let Some(inputs) = RootInputs::parse(&source) else {
        return Ok(false);
    };
    let mut entries: Vec<&InputEntry> = inputs
        .entries
        .iter()
        .filter(|entry| entry.name == name)
        .collect();
    if entries.is_empty() {
        return Ok(false);
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.start));

    let mut updated = source;
    for entry in entries {
        let (start, end) = removal_span(&updated, entry.start, entry.end);
        updated.replace_range(start..end, "");
    }
    fs::write(flake_file, updated).with_context(|| format!("writing {}", flake_file.display()))?;
    Ok(true)
}

/// The root input that points at `github:<repo>`, if any.
pub fn find_flake_input(flake_file: &Path, repo: &str) -> Result<Option<String>> {
    if !flake_file.exists() {
        return Ok(None);
    }
    let source = fs::read_to_string(flake_file)
        .with_context(|| format!("reading {}", flake_file.display()))?;
    Ok(RootInputs::parse(&source).and_then(|inputs| inputs.name_for_repo(repo).map(String::from)))
}

/// Every root input that points at a GitHub repository, as input name to
/// `owner/repo`.
pub fn input_repos(flake_file: &Path) -> BTreeMap<String, String> {
    let Ok(source) = fs::read_to_string(flake_file) else {
        return BTreeMap::new();
    };
    RootInputs::parse(&source)
        .map(|inputs| {
            inputs
                .entries
                .into_iter()
                .filter_map(|entry| Some((entry.name, entry.repo?)))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether anything other than nixbox's own bookkeeping still uses the root
/// input `name`: the flake's outputs, a `follows` pointing at it, or an
/// `inputs.<name>` reference in any `.nix` file under `config_dir`.
///
/// Removal consults this so that uninstalling a flake nixbox wired in never
/// drops an input a hand-written module also relies on.
pub fn input_referenced(config_dir: &Path, flake_file: &Path, name: &str) -> Result<bool> {
    let bare = name.trim_matches('"');
    if let Ok(source) = fs::read_to_string(flake_file) {
        if let Some(outputs) = root_binding(&source, "outputs")
            && contains_word(outputs, name)
        {
            return Ok(true);
        }
        if source.contains(&format!("follows = \"{bare}\""))
            || source.contains(&format!("follows = \"{bare}/"))
        {
            return Ok(true);
        }
    }
    references_under(config_dir, flake_file, &format!("inputs.{name}"))
}

/// Makes sure `setting` (for example `specialArgs`) exists in `file` and
/// inherits `inputs`. When the setting is missing it is added as the first
/// argument of `constructor`.
pub fn ensure_inputs_passed(file: &Path, constructor: &str, setting: &str) -> Result<()> {
    if pass_inputs_through(file, setting)? {
        return Ok(());
    }
    let source = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let constructor_pos = source
        .find(constructor)
        .with_context(|| format!("could not find `{constructor}` in {}", file.display()))?;
    let open = source[constructor_pos..]
        .find('{')
        .map(|offset| constructor_pos + offset)
        .context("could not find configuration arguments")?;
    let mut updated = source;
    updated.insert_str(
        open + 1,
        &format!("\n\t\t{setting} = {{ inherit inputs; }};"),
    );
    fs::write(file, updated).with_context(|| format!("writing {}", file.display()))
}

/// Adds `inherit inputs;` to an existing `setting = { ... };` in `file`.
/// Returns false when `file` has no such setting.
pub fn pass_inputs_through(file: &Path, setting: &str) -> Result<bool> {
    if !file.exists() {
        return Ok(false);
    }
    let source = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let Some(value) = setting_value(&source, setting) else {
        return Ok(false);
    };
    // Anything other than a literal set (a variable, `a // b`) is left alone:
    // there is no safe place to add to it.
    if source.as_bytes().get(value) != Some(&b'{') {
        return Ok(true);
    }
    let Some((close, _)) = scan_set(&source, value) else {
        return Ok(true);
    };
    if contains_word(&source[value..close], "inputs") {
        return Ok(true);
    }
    let mut updated = source;
    updated.insert_str(value + 1, " inherit inputs;");
    fs::write(file, updated).with_context(|| format!("writing {}", file.display()))?;
    Ok(true)
}

/// Passes `inputs` to Home Manager modules when Home Manager runs as a NixOS
/// module, by adding `extraSpecialArgs` to the `home-manager` options in
/// `file` (usually `configuration.nix`).
pub fn ensure_home_manager_special_args(file: &Path) -> Result<()> {
    if pass_inputs_through(file, "extraSpecialArgs")? {
        return ensure_module_arg(file, "inputs");
    }
    let source = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let mut updated = source.clone();
    if let Some(value) = setting_value(&source, "home-manager")
        && source.as_bytes().get(value) == Some(&b'{')
    {
        let indent = entry_indent(&source, value);
        updated.insert_str(
            value + 1,
            &format!("\n{indent}extraSpecialArgs = {{ inherit inputs; }};"),
        );
    } else if let Some(pos) = source.find("home-manager.users") {
        let line = line_start(&source, pos);
        let indent = &source[line..pos];
        updated.insert_str(
            line,
            &format!("{indent}home-manager.extraSpecialArgs = {{ inherit inputs; }};\n"),
        );
    } else {
        bail!(
            "could not find the `home-manager` options in {}; add \
             `home-manager.extraSpecialArgs = {{ inherit inputs; }};` by hand",
            file.display()
        );
    }
    fs::write(file, updated).with_context(|| format!("writing {}", file.display()))?;
    ensure_module_arg(file, "inputs")
}

/// Adds `arg` to the argument set of the module function in `file` when it is
/// not already there.
fn ensure_module_arg(file: &Path, arg: &str) -> Result<()> {
    let source = fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let Some(open) = first_token(&source, 0).filter(|&pos| source.as_bytes()[pos] == b'{') else {
        return Ok(());
    };
    let Some((close, _)) = scan_set(&source, open) else {
        return Ok(());
    };
    let is_function = source[close + 1..].trim_start().starts_with(':');
    if !is_function || contains_word(&source[open..close], arg) {
        return Ok(());
    }
    let mut updated = source;
    updated.insert_str(open + 1, &format!(" {arg},"));
    fs::write(file, updated).with_context(|| format!("writing {}", file.display()))
}

/// Whether the flake's outputs function binds the whole input set as
/// `inputs`, in either `inputs@{ ... }:`, `{ ... }@inputs:`, or `inputs:`
/// form.
fn outputs_bind_inputs(source: &str) -> bool {
    let Some(value) = root_binding(source, "outputs") else {
        return false;
    };
    let value = value.trim_start();
    if let Some(rest) = value.strip_prefix("inputs") {
        return rest.trim_start().starts_with(['@', ':']);
    }
    if !value.starts_with('{') {
        return false;
    }
    let Some((close, _)) = scan_set(value, 0) else {
        return false;
    };
    let Some(rest) = value[close + 1..].trim_start().strip_prefix('@') else {
        return false;
    };
    rest.trim_start()
        .strip_prefix("inputs")
        .is_some_and(|rest| !rest.starts_with(is_ident_char))
}

/// One binding that declares (part of) a root input, such as
/// `zen-browser = { ... };` or `inputs.zen-browser.url = "...";`.
#[derive(Debug)]
struct InputEntry {
    name: String,
    repo: Option<String>,
    start: usize,
    end: usize,
}

#[derive(Debug)]
enum InputsLayout {
    /// `inputs = { ... };`, with the positions of its braces.
    Block { open: usize, close: usize },
    /// `inputs.<name>... = ...;` bindings directly in the root set, whose
    /// `{` is at `root_open`.
    Dotted { root_open: usize },
}

#[derive(Debug)]
struct RootInputs {
    layout: InputsLayout,
    entries: Vec<InputEntry>,
}

impl RootInputs {
    fn parse(source: &str) -> Option<Self> {
        let root_open = first_token(source, 0).filter(|&pos| source.as_bytes()[pos] == b'{')?;
        let (_, root) = scan_set(source, root_open)?;

        for binding in &root {
            if binding.path(source) != ["inputs"] {
                continue;
            }
            let value = binding.value(source)?;
            if source.as_bytes()[value] != b'{' {
                return None;
            }
            let (close, bindings) = scan_set(source, value)?;
            let entries = bindings
                .iter()
                .filter_map(|binding| binding.as_input(source, 0))
                .collect();
            return Some(Self {
                layout: InputsLayout::Block { open: value, close },
                entries,
            });
        }

        let entries = root
            .iter()
            .filter(|binding| binding.path(source).first().map(String::as_str) == Some("inputs"))
            .filter_map(|binding| binding.as_input(source, 1))
            .collect();
        Some(Self {
            layout: InputsLayout::Dotted { root_open },
            entries,
        })
    }

    fn name_for_repo(&self, repo: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| {
                entry
                    .repo
                    .as_deref()
                    .is_some_and(|found| found.eq_ignore_ascii_case(repo))
            })
            .map(|entry| entry.name.as_str())
    }

    fn unused_name(&self, repo: &str) -> String {
        let taken = |name: &str| self.entries.iter().any(|entry| entry.name == name);
        let base = input_name_for(repo);
        if !taken(&base) {
            return base;
        }
        let owner = repo.split('/').next().unwrap_or_default();
        let owned = attr_name(&format!("{owner}-{}", base.trim_matches('"')));
        if !taken(&owned) {
            return owned;
        }
        (2..)
            .map(|n| attr_name(&format!("{}-{n}", base.trim_matches('"'))))
            .find(|name| !taken(name))
            .unwrap_or(base)
    }

    fn insert(&self, source: &str, name: &str, repo: &str, follows: bool) -> String {
        let spaced = self.entries.windows(2).any(|pair| {
            pair[0].name != pair[1].name
                && source[pair[0].end..pair[1].start].matches('\n').count() >= 2
        });
        let mut updated = source.to_string();
        match self.layout {
            InputsLayout::Block { open, close } => {
                let (indent, unit) = self.indentation(source, open);
                let mut text = String::new();
                if spaced {
                    text.push('\n');
                }
                text.push_str(&input_entry(&indent, &unit, name, repo, follows));
                let close_line = line_start(source, close);
                if source[close_line..close].trim().is_empty() {
                    updated.insert_str(close_line, &text);
                } else {
                    let outer = leading_indent(source, open);
                    updated.insert_str(close, &format!("\n{text}{outer}"));
                }
            }
            InputsLayout::Dotted { root_open } => {
                let (indent, unit) = self.indentation(source, root_open);
                let entry = input_entry(&indent, &unit, &format!("inputs.{name}"), repo, follows);
                match self.entries.last() {
                    Some(last) => {
                        let at = line_end(source, last.end);
                        let text = if spaced { format!("\n{entry}") } else { entry };
                        updated.insert_str(at, &text);
                    }
                    None => {
                        let at = line_end(source, root_open + 1);
                        updated.insert_str(at, &entry);
                    }
                }
            }
        }
        updated
    }

    /// The indentation of existing entries and one indentation step, falling
    /// back to tabs when the block has no entries to copy from.
    fn indentation(&self, source: &str, open: usize) -> (String, String) {
        let outer = leading_indent(source, open);
        let Some(first) = self.entries.first() else {
            return (format!("{outer}\t"), "\t".into());
        };
        let line = line_start(source, first.start);
        let indent = &source[line..first.start];
        if !indent.trim().is_empty() {
            return (format!("{outer}\t"), "\t".into());
        }
        let unit = indent.strip_prefix(outer.as_str()).unwrap_or("\t");
        let unit = if unit.is_empty() { "\t" } else { unit };
        (indent.to_string(), unit.to_string())
    }
}

fn input_entry(indent: &str, unit: &str, name: &str, repo: &str, follows: bool) -> String {
    if follows {
        format!(
            "{indent}{name} = {{\n{indent}{unit}url = \"github:{repo}\";\n{indent}{unit}inputs.nixpkgs.follows = \"nixpkgs\";\n{indent}}};\n"
        )
    } else {
        format!("{indent}{name}.url = \"github:{repo}\";\n")
    }
}

/// A short input name for `owner/repo`, the way people usually name them by
/// hand: `0xc000022070/zen-browser-flake` becomes `zen-browser`,
/// `oxcl/nix-flake-helium-browser` becomes `helium-browser`, and
/// `numtide/llm-agents.nix` becomes `llm-agents`.
pub fn input_name_for(repo: &str) -> String {
    let full = repo.rsplit('/').next().unwrap_or(repo);
    let mut name = full.strip_suffix(".nix").unwrap_or(full);
    for suffix in ["-flake", "_flake", ".flake"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    for prefix in ["nix-flake-", "flake-"] {
        name = name.strip_prefix(prefix).unwrap_or(name);
    }
    if name.is_empty() {
        name = full;
    }
    attr_name(&name.replace('.', "-"))
}

/// `name` as a Nix attribute name, quoted when it is not a plain identifier.
pub fn attr_name(name: &str) -> String {
    const KEYWORDS: [&str; 10] = [
        "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
    ];
    let plain = name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && name.chars().all(is_ident_char)
        && !KEYWORDS.contains(&name);
    if plain {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// A `path = value;` binding directly inside an attribute set, as byte
/// offsets into the source. `end` is just past the `;`.
#[derive(Debug)]
struct Binding {
    start: usize,
    end: usize,
}

impl Binding {
    fn equals(&self, source: &str) -> Option<usize> {
        source[self.start..self.end]
            .find('=')
            .map(|offset| self.start + offset)
    }

    fn path(&self, source: &str) -> Vec<String> {
        let lhs = match self.equals(source) {
            Some(eq) => &source[self.start..eq],
            None => &source[self.start..self.end],
        };
        split_attr_path(lhs.trim())
    }

    /// Offset of the first token of the bound value.
    fn value(&self, source: &str) -> Option<usize> {
        first_token(source, self.equals(source)? + 1).filter(|&pos| pos < self.end)
    }

    fn as_input(&self, source: &str, skip: usize) -> Option<InputEntry> {
        let name = self.path(source).into_iter().nth(skip)?;
        Some(InputEntry {
            name,
            repo: github_repo(&source[self.start..self.end]),
            start: self.start,
            end: self.end,
        })
    }
}

/// The owner and repository of the first `github:` reference in `text`.
fn github_repo(text: &str) -> Option<String> {
    let start = text.find("\"github:")? + "\"github:".len();
    let reference = &text[start..];
    let reference = &reference[..reference.find(['"', '?']).unwrap_or(reference.len())];
    let mut parts = reference.split('/');
    let owner = parts.next().filter(|part| !part.is_empty())?;
    let repo = parts.next().filter(|part| !part.is_empty())?;
    Some(format!("{owner}/{repo}"))
}

fn split_attr_path(lhs: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in lhs.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            '.' if !quoted => parts.push(std::mem::take(&mut current).trim().to_string()),
            _ => current.push(ch),
        }
    }
    parts.push(current.trim().to_string());
    parts
}

/// The source text of the value bound to `name` in the file's root set.
fn root_binding<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let open = first_token(source, 0).filter(|&pos| source.as_bytes()[pos] == b'{')?;
    let (_, bindings) = scan_set(source, open)?;
    let binding = bindings
        .iter()
        .find(|binding| binding.path(source) == [name])?;
    Some(&source[binding.value(source)?..binding.end - 1])
}

/// Offset of the value of the first `setting = ...` in `source`, wherever it
/// is nested.
fn setting_value(source: &str, setting: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = source[from..].find(setting) {
        let pos = from + offset;
        from = pos + setting.len();
        let before = source[..pos].chars().next_back();
        if before.is_some_and(is_ident_char) {
            continue;
        }
        let rest = source[from..].trim_start();
        if rest.starts_with('=') && !rest.starts_with("==") {
            let eq = source.len() - rest.len();
            return first_token(source, eq + 1);
        }
    }
    None
}

/// Walks the attribute set whose `{` is at `open` and returns the offset of
/// its `}` together with its top-level bindings. Strings and comments are
/// skipped so braces and semicolons inside them do not count.
fn scan_set(source: &str, open: usize) -> Option<(usize, Vec<Binding>)> {
    let bytes = source.as_bytes();
    let mut bindings = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut i = open + 1;
    while i < bytes.len() {
        let ch = bytes[i];
        match ch {
            b'#' => {
                i = source[i..]
                    .find('\n')
                    .map_or(bytes.len(), |offset| i + offset);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = source[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |offset| i + 2 + offset + 2);
                continue;
            }
            b'"' => {
                if depth == 0 {
                    start.get_or_insert(i);
                }
                i = skip_string(bytes, i + 1);
                continue;
            }
            b'\'' if bytes.get(i + 1) == Some(&b'\'') => {
                if depth == 0 {
                    start.get_or_insert(i);
                }
                i = skip_indented_string(bytes, i + 2);
                continue;
            }
            b'{' | b'[' | b'(' => {
                if depth == 0 {
                    start.get_or_insert(i);
                }
                depth += 1;
            }
            b'}' | b']' | b')' => {
                if depth == 0 {
                    return (ch == b'}').then_some((i, bindings));
                }
                depth -= 1;
            }
            b';' if depth == 0 => {
                if let Some(start) = start.take() {
                    bindings.push(Binding { start, end: i + 1 });
                }
            }
            _ if ch.is_ascii_whitespace() => {}
            _ => {
                if depth == 0 {
                    start.get_or_insert(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Index just past the closing quote of a `"..."` string whose body starts
/// at `i`.
fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            b'$' if bytes.get(i + 1) == Some(&b'{') => i = skip_interpolation(bytes, i + 2),
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Index just past the closing `''` of an indented string whose body starts
/// at `i`.
fn skip_indented_string(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == b'\'' && bytes.get(i + 1) == Some(&b'\'') {
            match bytes.get(i + 2) {
                Some(b'$' | b'\'' | b'\\') => i += 3,
                _ => return i + 2,
            }
        } else if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
            i = skip_interpolation(bytes, i + 2);
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn skip_interpolation(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_string(bytes, i + 1);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Offset of the first character at or after `from` that is not whitespace
/// or part of a comment.
fn first_token(source: &str, mut from: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    while from < bytes.len() {
        match bytes[from] {
            ch if ch.is_ascii_whitespace() => from += 1,
            b'#' => {
                from = source[from..]
                    .find('\n')
                    .map_or(bytes.len(), |offset| from + offset)
            }
            b'/' if bytes.get(from + 1) == Some(&b'*') => {
                from = source[from + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |offset| from + 2 + offset + 2);
            }
            _ => return Some(from),
        }
    }
    None
}

/// The line span to delete for the binding at `start..end`: whole lines when
/// the binding sits on lines of its own, together with one of the blank lines
/// that separated it from its neighbours.
fn removal_span(source: &str, start: usize, end: usize) -> (usize, usize) {
    let line = line_start(source, start);
    let after = line_end(source, end);
    if !source[line..start].trim().is_empty() || !source[end..after].trim().is_empty() {
        return (start, end);
    }
    if line > 0 {
        let previous = line_start(source, line - 1);
        if source[previous..line].trim().is_empty() {
            return (previous, after);
        }
    }
    let next = line_end(source, after);
    if after < source.len() && source[after..next].trim().is_empty() {
        return (line, next);
    }
    (line, after)
}

fn line_start(source: &str, pos: usize) -> usize {
    source[..pos].rfind('\n').map_or(0, |offset| offset + 1)
}

/// Offset just past the newline that ends the line containing `pos`.
fn line_end(source: &str, pos: usize) -> usize {
    source[pos..]
        .find('\n')
        .map_or(source.len(), |offset| pos + offset + 1)
}

fn leading_indent(source: &str, pos: usize) -> String {
    let line = line_start(source, pos);
    source[line..]
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect()
}

/// The indentation for a new first line inside the set opened at `open`.
fn entry_indent(source: &str, open: usize) -> String {
    let next = line_end(source, open);
    if let Some(token) = first_token(source, next) {
        let indent = &source[line_start(source, token)..token];
        if indent.trim().is_empty() && !indent.is_empty() {
            return indent.to_string();
        }
    }
    format!("{}\t", leading_indent(source, open))
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '\'')
}

fn contains_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(pos, _)| {
        let before = text[..pos].chars().next_back();
        let after = text[pos + word.len()..].chars().next();
        !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
    })
}

/// Whether any `.nix` file under `dir` other than `skip` contains `needle`
/// as a whole attribute reference. Hidden directories and symlinks (such as
/// `result`) are not followed.
fn references_under(dir: &Path, skip: &Path, needle: &str) -> Result<bool> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(false),
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if entry.file_name().to_string_lossy().starts_with('.') || file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if references_under(&path, skip, needle)? {
                return Ok(true);
            }
        } else if path.extension().is_some_and(|ext| ext == "nix")
            && path != skip
            && fs::read_to_string(&path).is_ok_and(|source| {
                source.match_indices(needle).any(|(pos, _)| {
                    !source[pos + needle.len()..]
                        .chars()
                        .next()
                        .is_some_and(is_ident_char)
                })
            })
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_FLAKE: &str = "{\n    description = \"NixOS flake\";\n\n    inputs = {\n        nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";\n\n        zen-browser = {\n            url = \"github:0xc000022070/zen-browser-flake\";\n            inputs.nixpkgs.follows = \"nixpkgs\";\n        };\n\n        llm-agents.url = \"github:numtide/llm-agents.nix\";\n    };\n\n    outputs = { self, nixpkgs, home-manager, ... }@inputs: {\n        nixosConfigurations.\"nixos\" = nixpkgs.lib.nixosSystem {\n            specialArgs = { inherit inputs; };\n            modules = [ ./configuration.nix ];\n        };\n    };\n}\n";

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nixbox-wiring-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_inputs_the_way_people_do_by_hand() {
        assert_eq!(
            input_name_for("0xc000022070/zen-browser-flake"),
            "zen-browser"
        );
        assert_eq!(
            input_name_for("oxcl/nix-flake-helium-browser"),
            "helium-browser"
        );
        assert_eq!(input_name_for("numtide/llm-agents.nix"), "llm-agents");
        assert_eq!(input_name_for("ryoppippi/nix-bun"), "nix-bun");
        assert_eq!(input_name_for("owner/1password"), "\"1password\"");
    }

    #[test]
    fn accepts_both_ways_of_binding_inputs() {
        assert!(outputs_bind_inputs(ROOT_FLAKE));
        assert!(outputs_bind_inputs(
            "{ inputs = {}; outputs = inputs@{ self, ... }: {}; }"
        ));
        assert!(outputs_bind_inputs(
            "{ inputs = {}; outputs = inputs: {}; }"
        ));
        assert!(!outputs_bind_inputs(
            "{ inputs = {}; outputs = { self, nixpkgs }: {}; }"
        ));
    }

    #[test]
    fn reuses_an_input_that_already_points_at_the_repository() {
        let dir = temp_dir("reuse");
        let file = dir.join("flake.nix");
        fs::write(&file, ROOT_FLAKE).unwrap();

        let name = ensure_flake_input(&file, "0xC000022070/zen-browser-flake").unwrap();

        assert_eq!(name, "zen-browser");
        assert_eq!(fs::read_to_string(&file).unwrap(), ROOT_FLAKE);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn adds_a_named_input_that_follows_nixpkgs_in_the_file_style() {
        let dir = temp_dir("add");
        let file = dir.join("flake.nix");
        fs::write(&file, ROOT_FLAKE).unwrap();

        let name = ensure_flake_input(&file, "oxcl/nix-flake-helium-browser").unwrap();

        assert_eq!(name, "helium-browser");
        let updated = fs::read_to_string(&file).unwrap();
        assert!(updated.contains(
            "        llm-agents.url = \"github:numtide/llm-agents.nix\";\n\n        helium-browser = {\n            url = \"github:oxcl/nix-flake-helium-browser\";\n            inputs.nixpkgs.follows = \"nixpkgs\";\n        };\n    };\n"
        ));

        assert!(remove_flake_input(&file, "helium-browser").unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), ROOT_FLAKE);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn avoids_a_name_another_repository_already_uses() {
        let dir = temp_dir("collide");
        let file = dir.join("flake.nix");
        fs::write(&file, ROOT_FLAKE).unwrap();

        let name = ensure_flake_input(&file, "someone/zen-browser").unwrap();

        assert_eq!(name, "someone-zen-browser");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn handles_dotted_inputs_at_the_root() {
        let dir = temp_dir("dotted");
        let file = dir.join("flake.nix");
        fs::write(
            &file,
            "{\n\tinputs.nixpkgs.url = \"github:NixOS/nixpkgs\";\n\toutputs = inputs@{ nixpkgs, ... }: { };\n}\n",
        )
        .unwrap();

        let name = ensure_flake_input(&file, "ryoppippi/nix-bun").unwrap();

        assert_eq!(name, "nix-bun");
        let updated = fs::read_to_string(&file).unwrap();
        assert!(updated.contains(
            "\tinputs.nixpkgs.url = \"github:NixOS/nixpkgs\";\n\tinputs.nix-bun = {\n\t\turl = \"github:ryoppippi/nix-bun\";"
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn passes_inputs_to_home_manager_as_a_nixos_module() {
        let dir = temp_dir("hm");
        let file = dir.join("configuration.nix");
        fs::write(
            &file,
            "{ config, pkgs, ... }:\n\n{\n    home-manager = {\n        useGlobalPkgs = true;\n        users.me = import ./home.nix;\n    };\n}\n",
        )
        .unwrap();

        ensure_home_manager_special_args(&file).unwrap();

        let updated = fs::read_to_string(&file).unwrap();
        assert!(updated.starts_with("{ inputs, config, pkgs, ... }:"));
        assert!(updated.contains(
            "    home-manager = {\n        extraSpecialArgs = { inherit inputs; };\n        useGlobalPkgs = true;"
        ));

        ensure_home_manager_special_args(&file).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), updated);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn adds_inputs_to_an_existing_special_args_set() {
        let dir = temp_dir("special");
        let file = dir.join("flake.nix");
        fs::write(
            &file,
            "{\n  outputs = inputs@{ nixpkgs, ... }: {\n    homeConfigurations.me = inputs.home-manager.lib.homeManagerConfiguration {\n      extraSpecialArgs = { host = \"box\"; };\n    };\n  };\n}\n",
        )
        .unwrap();

        ensure_inputs_passed(&file, "homeManagerConfiguration", "extraSpecialArgs").unwrap();

        assert!(
            fs::read_to_string(&file)
                .unwrap()
                .contains("extraSpecialArgs = { inherit inputs; host = \"box\"; };")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_inputs_that_hand_written_modules_still_use() {
        let dir = temp_dir("refs");
        let file = dir.join("flake.nix");
        fs::write(&file, ROOT_FLAKE).unwrap();
        fs::write(
            dir.join("home.nix"),
            "{ inputs, pkgs, ... }: { home.packages = [ inputs.zen-browser.packages.x.default ]; }\n",
        )
        .unwrap();

        assert!(input_referenced(&dir, &file, "zen-browser").unwrap());
        assert!(!input_referenced(&dir, &file, "llm-agents").unwrap());
        assert!(input_referenced(&dir, &file, "home-manager").unwrap());
        let _ = fs::remove_dir_all(dir);
    }
}
