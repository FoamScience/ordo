// ------------------------------------------------------------------ config UI
use crate::code_view::theme;
use crate::code_view::theme_names;
use crate::code_view::Theme;
use crate::commands::run_strategy;
use crate::draw::centred;
use crate::highlight::highlight_file;
use crate::keys::action_help;
use crate::keys::apply_theme_colors;
use crate::keys::category_label;
use crate::keys::keymap;
use crate::keys::parse_hex;
use crate::keys::theme_role_color;
use crate::keys::Category;
use crate::keys::Key;
use crate::keys::Keymap;
use crate::keys::ACTION_NAMES;
use crate::keys::KEYMAP_NAMES;
use crate::keys::THEME_ROLES;
use crate::marks::write_atomic;
use crate::rules::init_config;
use crate::rules::user_rules_path;
use crate::rules::PRESETS;
use crate::App;
use crate::Popup;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

/// What a setting is, which decides how `:config` shows and changes it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum FieldKind {
    /// on/off — Space toggles
    Flag(bool),
    /// one of a fixed set — Space cycles
    Choice { at: usize, options: Vec<String> },
    /// free text (a colour, a key binding) — Enter opens it in the command bar
    Text(String),
}

/// One setting: where it is written, what it is now, and what it means.
#[derive(Clone, Debug)]
pub(super) struct ConfigField {
    /// the config key this writes, e.g. `theme.accent`, `binds.j`, `catalog`
    pub(super) key: String,
    pub(super) label: String,
    pub(super) help: String,
    pub(super) kind: FieldKind,
}

#[derive(Clone, Debug)]
pub(super) struct ConfigSection {
    pub(super) title: String,
    /// which file this section is written to, shown in the UI so a reviewer
    /// knows what `:config` is about to edit
    pub(super) file: String,
    pub(super) fields: Vec<ConfigField>,
}

/// The whole settings surface, derived from the tables the program already
/// reads: `KEYMAP_NAMES`, `theme_names`, `THEME_ROLES`, the live `Keymap`,
/// `ACTION_NAMES`, `PRESETS` and `ordo::catalog::sections`.
///
/// Nothing here is a hand-written list. A new theme role, key action, bundled
/// ruleset or catalog rule appears in `:config` because it appears in the table
/// it already had to be added to.
pub(super) fn config_schema(app: &App) -> Vec<ConfigSection> {
    // the header names the file `w` will actually write. With no config
    // directory there is no such file, and saying so beats printing a bare
    // name that implies one — the writer would silently skip it
    let unresolved = "(no config directory — $XDG_CONFIG_HOME or $HOME unset)";
    let tui = config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| unresolved.to_string());
    let rules_file = user_rules_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| unresolved.to_string());
    let disabled = |name: &str| app.disables.iter().any(|d| d == name);

    let mut out = vec![];

    let choice = |current: &str, options: Vec<String>| {
        let at = options.iter().position(|o| o == current).unwrap_or(0);
        FieldKind::Choice { at, options }
    };
    out.push(ConfigSection {
        title: "general".to_string(),
        file: tui.clone(),
        fields: vec![
            ConfigField {
                key: "preset".to_string(),
                label: "keymap preset".to_string(),
                help: "which set of bindings the reviewer starts from".to_string(),
                kind: choice(
                    app.keys.name,
                    KEYMAP_NAMES.iter().map(|s| s.to_string()).collect(),
                ),
            },
            ConfigField {
                key: "docs_last".to_string(),
                label: "docs read last".to_string(),
                help: "a docs-only hunk sorts after the code it describes".to_string(),
                kind: FieldKind::Flag(app.docs_last),
            },
            ConfigField {
                key: "theme".to_string(),
                label: "theme".to_string(),
                help: "the palette; the roles below override it".to_string(),
                kind: choice(app.theme.name, theme_names()),
            },
        ],
    });

    // the construct catalog: the whole thing, then a switch per file, then one
    // per rule — a reviewer turns off "the C++ constructs", not nineteen names
    let mut catalog = vec![ConfigField {
        key: "catalog".to_string(),
        label: "construct catalog".to_string(),
        help: "the built-in advisories; off leaves only your own rules".to_string(),
        kind: FieldKind::Flag(app.catalog),
    }];
    for sec in ordo::catalog::sections() {
        let all_off = sec.rules.iter().all(|r| disabled(r));
        catalog.push(ConfigField {
            key: format!("section:{}", sec.name),
            label: format!("  {} ({} rules)", sec.name, sec.rules.len()),
            help: "every rule in this file".to_string(),
            kind: FieldKind::Flag(!all_off),
        });
        for r in &sec.rules {
            catalog.push(ConfigField {
                key: format!("disable:{r}"),
                label: format!("    {r}"),
                help: String::new(),
                kind: FieldKind::Flag(!disabled(r)),
            });
        }
    }
    out.push(ConfigSection {
        title: "catalog".to_string(),
        file: rules_file.clone(),
        fields: catalog,
    });

    out.push(ConfigSection {
        title: "rulesets".to_string(),
        file: rules_file,
        fields: PRESETS
            .iter()
            .map(|(name, _)| ConfigField {
                key: format!("include:{name}"),
                label: name.to_string(),
                help: "a bundled convention set".to_string(),
                kind: FieldKind::Flag(app.includes.iter().any(|i| i == name)),
            })
            .collect(),
    });

    // the live theme is its palette with the reviewer's overrides already laid
    // on top, so a role that differs from the bare palette is an override and
    // one that matches is not — which is the distinction a row has to show, or
    // swapping the theme would carry the old palette's colours over with it
    let bare = theme(app.theme.name);
    out.push(ConfigSection {
        title: "theme roles".to_string(),
        file: tui.clone(),
        fields: THEME_ROLES
            .iter()
            .map(|role| {
                let cur = theme_role_color(&app.theme, role);
                let overridden = bare
                    .as_ref()
                    .is_none_or(|b| theme_role_color(b, role) != cur);
                ConfigField {
                    key: format!("theme.{role}"),
                    label: role.to_string(),
                    help: "#rrggbb, or empty to follow the theme".to_string(),
                    kind: FieldKind::Text(match cur {
                        Color::Rgb(r, g, b) if overridden => format!("#{r:02x}{g:02x}{b:02x}"),
                        _ => String::new(),
                    }),
                }
            })
            .collect(),
    });

    out.push(ConfigSection {
        title: "keys".to_string(),
        file: tui,
        fields: app
            .keys
            .binds
            .iter()
            .map(|(prefix, key, action)| {
                let keys = match prefix {
                    Some(p) => format!("{} {}", key_label(*p), key_label(*key)),
                    None => key_label(*key),
                };
                let name = ACTION_NAMES
                    .iter()
                    .find(|(_, a)| a == action)
                    .map(|(n, _)| *n)
                    .unwrap_or("");
                ConfigField {
                    key: format!("binds.{keys}"),
                    label: keys.clone(),
                    help: action_help(*action).1.to_string(),
                    kind: FieldKind::Text(name.to_string()),
                }
            })
            .collect(),
    });
    out
}

/// `:config`'s open state: the generated schema, plus where the cursor is.
///
/// The schema is rebuilt on open rather than cached, so it always shows what
/// the session actually holds.
pub(super) struct ConfigUi {
    pub(super) sections: Vec<ConfigSection>,
    /// the schema as it was on open, so writing touches only what changed —
    /// flipping one rule must not materialise all 47 binds into the file
    pub(super) original: Vec<ConfigSection>,
    /// (section, field) for every navigable row, in display order
    pub(super) rows: Vec<(usize, usize)>,
    pub(super) sel: usize,
    pub(super) scroll: u16,
    /// something changed and has not been written
    pub(super) dirty: bool,
    /// the text being typed into the selected field; `None` unless editing.
    /// A colour and a key binding are free text, and a settings view that can
    /// show them but not change them is a list, not a config.
    pub(super) editing: Option<String>,
}

impl ConfigUi {
    pub(super) fn open(app: &App) -> ConfigUi {
        let sections = config_schema(app);
        let rows = sections
            .iter()
            .enumerate()
            .flat_map(|(i, s)| (0..s.fields.len()).map(move |j| (i, j)))
            .collect();
        ConfigUi {
            original: sections.clone(),
            sections,
            rows,
            sel: 0,
            scroll: 0,
            dirty: false,
            editing: None,
        }
    }

    /// Every field whose value differs from the one this view opened with.
    pub(super) fn changed(&self) -> Vec<&ConfigField> {
        let was: HashMap<&str, &FieldKind> = self
            .original
            .iter()
            .flat_map(|s| &s.fields)
            .map(|f| (f.key.as_str(), &f.kind))
            .collect();
        self.sections
            .iter()
            .flat_map(|s| &s.fields)
            .filter(|f| was.get(f.key.as_str()).is_some_and(|k| **k != f.kind))
            .collect()
    }

    pub(super) fn field(&self, at: usize) -> Option<&ConfigField> {
        let (i, j) = *self.rows.get(at)?;
        self.sections.get(i)?.fields.get(j)
    }

    pub(super) fn field_mut(&mut self, at: usize) -> Option<&mut ConfigField> {
        let (i, j) = *self.rows.get(at)?;
        self.sections.get_mut(i)?.fields.get_mut(j)
    }

    /// Space on the selected row: flip a flag, cycle a choice. Text fields are
    /// edited through the command bar instead, which already does text.
    ///
    /// A section's switch carries its rules with it — turning off "the C++
    /// constructs" is the gesture, not fourteen of them — and a rule turned off
    /// by hand leaves the section showing off once nothing in it is left on.
    pub(super) fn toggle(&mut self) -> bool {
        let Some(f) = self.field_mut(self.sel) else {
            return false;
        };
        let (key, now) = (f.key.clone(), f.kind.clone());
        match &mut f.kind {
            FieldKind::Flag(v) => *v = !*v,
            FieldKind::Choice { at, options } => {
                if !options.is_empty() {
                    *at = (*at + 1) % options.len();
                }
            }
            // text opens for editing rather than cycling
            FieldKind::Text(t) => {
                let seed = t.clone();
                self.editing = Some(seed);
                return false;
            }
        }
        if let (Some(section), FieldKind::Flag(was)) = (key.strip_prefix("section:"), now) {
            self.set_section(section, !was);
        }
        self.sync_sections();
        self.dirty = true;
        true
    }

    /// Commit the text being typed into the selected field.
    pub(super) fn commit_edit(&mut self) {
        let Some(text) = self.editing.take() else {
            return;
        };
        let sel = self.sel;
        if let Some(f) = self.field_mut(sel) {
            if let FieldKind::Text(t) = &mut f.kind {
                if *t != text {
                    *t = text;
                    self.dirty = true;
                }
            }
        }
    }

    /// Set every rule of one catalog section.
    pub(super) fn set_section(&mut self, section: &str, on: bool) {
        let names: Vec<String> = ordo::catalog::sections()
            .iter()
            .find(|s| s.name == section)
            .map(|s| s.rules.clone())
            .unwrap_or_default();
        for sec in &mut self.sections {
            for f in &mut sec.fields {
                if let Some(rule) = f.key.strip_prefix("disable:") {
                    if names.iter().any(|n| n == rule) {
                        f.kind = FieldKind::Flag(on);
                    }
                }
            }
        }
    }

    /// A section reads on while any of its rules is on.
    pub(super) fn sync_sections(&mut self) {
        let on: Vec<(String, bool)> = ordo::catalog::sections()
            .iter()
            .map(|s| {
                let any = s.rules.iter().any(|r| {
                    self.sections.iter().flat_map(|x| &x.fields).any(|f| {
                        f.key.strip_prefix("disable:") == Some(r.as_str())
                            && matches!(f.kind, FieldKind::Flag(true))
                    })
                });
                (s.name.clone(), any)
            })
            .collect();
        for sec in &mut self.sections {
            for f in &mut sec.fields {
                if let Some(name) = f.key.strip_prefix("section:") {
                    if let Some((_, any)) = on.iter().find(|(n, _)| n == name) {
                        f.kind = FieldKind::Flag(*any);
                    }
                }
            }
        }
    }
}

/// Set `key = value` under `[section]`, preserving everything else in the file
/// — including the commented-out defaults `--init-config` writes, which are the
/// documentation.
///
/// An existing line for the key is replaced where it sits, commented or not, so
/// a setting keeps its place and its explaining comment. Otherwise the line is
/// appended to the section, and the section is created if it is missing.
pub(super) fn upsert_toml(text: &str, section: Option<&str>, key: &str, value: &str) -> String {
    let want = format!("{key} = {value}");
    let mut out: Vec<String> = vec![];
    let mut here = section.is_none();
    let mut done = false;
    let mut last_in_section = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(head) = t.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            here = section.is_some_and(|s| s == head.trim());
        }
        // a key line, live or commented out, is the one to replace
        let bare = t.trim_start_matches('#').trim();
        let is_key = !done
            && here
            && bare
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == key || k.trim().trim_matches('"') == key);
        if is_key {
            out.push(want.clone());
            done = true;
        } else {
            out.push(line.to_string());
        }
        if here {
            last_in_section = Some(out.len());
        }
    }
    if !done {
        match (section, last_in_section) {
            // append inside the section it belongs to
            (Some(_), Some(at)) => out.insert(at, want),
            (None, _) => out.insert(0, want),
            (Some(s), None) => {
                out.push(String::new());
                out.push(format!("[{s}]"));
                out.push(want);
            }
        }
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// `--init-config`: write the generated config, refusing to clobber one that
/// already exists unless asked. Reports the path either way — the file is no
/// use if the reviewer can't find it.
pub(super) fn write_init_config(preset: &str, theme_name: &str, force: bool) -> Result<(), i32> {
    let Some(path) = config_path() else {
        eprintln!("ordo: no config directory (set $XDG_CONFIG_HOME or $HOME)");
        return Err(2);
    };
    if path.exists() && !force {
        eprintln!(
            "ordo: {} already exists — pass --force to overwrite it",
            path.display()
        );
        return Err(1);
    }
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("ordo: {}: {e}", dir.display());
            return Err(1);
        }
    }
    match std::fs::write(&path, init_config(preset, theme_name)) {
        Ok(()) => {
            println!("wrote {}", path.display());
            Err(0)
        }
        Err(e) => {
            eprintln!("ordo: {}: {e}", path.display());
            Err(1)
        }
    }
}

pub(super) fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ordo").join("tui.toml"))
}

/// A config line without its trailing comment. `#` opens a comment only
/// *outside* quotes: a colour is written `"#89b4fa"`, and cutting at the first
/// `#` regardless would eat every palette value in the file.
pub(super) fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '#') => return &line[..i],
            _ => {}
        }
    }
    line
}

/// One key, rendered legibly: `C-`/`S-`/`A-` modifier prefixes, named special
/// keys, `F<n>` for function keys, the char itself otherwise.
pub(super) fn key_label(key: Key) -> String {
    let (code, mods) = key;
    let mut prefix = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        prefix.push_str("C-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        prefix.push_str("S-");
    }
    if mods.contains(KeyModifiers::ALT) {
        prefix.push_str("A-");
    }
    let body = match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        other => format!("{other:?}"),
    };
    format!("{prefix}{body}")
}

/// A bind's full chord: `gg`/`ge` (no space — vim's own convention for a
/// plain-char chord) vs. `C-w C-w` (spaced — either half carries a modifier
/// or a named key, and vim always writes those chords spaced).
pub(super) fn chord_label(prefix: Option<Key>, key: Key) -> String {
    let Some(p) = prefix else {
        return key_label(key);
    };
    let simple = |k: Key| matches!(k.0, KeyCode::Char(_)) && k.1 == KeyModifiers::NONE;
    if simple(p) && simple(key) {
        format!("{}{}", key_label(p), key_label(key))
    } else {
        format!("{} {}", key_label(p), key_label(key))
    }
}

/// The `?`/`F1` help popup body: every bind in `keys`, grouped by category and
/// collapsed onto one row per action (several keys can mean the same thing,
/// e.g. `j` and `Down` both `Next`) — generated straight from `Keymap.binds`
/// rather than hand-duplicated, so it cannot describe a key the table doesn't
/// actually bind.
pub(super) fn build_help(keys: &Keymap) -> Vec<String> {
    struct Row {
        category: Category,
        desc: &'static str,
        keys: Vec<String>,
    }
    let mut rows: Vec<Row> = vec![];
    for &(prefix, key, action) in &keys.binds {
        let (category, desc) = action_help(action);
        let label = chord_label(prefix, key);
        match rows
            .iter_mut()
            .find(|r| r.category == category && r.desc == desc)
        {
            Some(r) if !r.keys.contains(&label) => r.keys.push(label),
            Some(_) => {}
            None => rows.push(Row {
                category,
                desc,
                keys: vec![label],
            }),
        }
    }
    let order = [
        Category::General,
        Category::Navigation,
        Category::Panes,
        Category::Search,
        Category::Review,
        Category::Editor,
        Category::Help,
    ];
    let mut out = vec![];
    for &cat in &order {
        let group: Vec<&Row> = rows.iter().filter(|r| r.category == cat).collect();
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(category_label(cat).to_uppercase());
        // At most three chords per row. The help is for discovery, and one
        // action with four aliases (`Space, f, C-f, PageDown`) widened the key
        // column for every other row in its section; the full set is in the
        // generated table in docs/tui.md.
        const SHOWN: usize = 3;
        let label = |r: &Row| {
            let mut l = r.keys[..r.keys.len().min(SHOWN)].join(", ");
            if r.keys.len() > SHOWN {
                l.push('…');
            }
            l
        };
        // the column is as wide as the widest chord in this section, not a
        // fixed 16 that the longest row overflowed and fell out of line with
        let w = group
            .iter()
            .map(|r| label(r).chars().count())
            .max()
            .unwrap_or(0);
        for r in group {
            out.push(format!("  {:<w$} {}", label(r), r.desc));
        }
    }
    out
}

// ------------------------------------------------------------ config screen

/// Move `:config`'s selection, clamped.
pub(super) fn move_config(app: &mut App, by: isize) {
    let Some(c) = app.config.as_mut() else { return };
    if c.rows.is_empty() {
        return;
    }
    let last = c.rows.len() as isize - 1;
    c.sel = (c.sel as isize).saturating_add(by).clamp(0, last) as usize;
}

/// Space on the selected setting. A rule or catalog change re-runs the engine,
/// because what the reviewer is looking at is the answer to those settings.
/// Swap the palette. Syntax colours are baked into the highlight cache at load
/// time, so what is already on screen has to be highlighted again.
pub(super) fn swap_theme(app: &mut App, t: Theme) {
    app.theme = t;
    let paths: Vec<String> = app.highlights.keys().cloned().collect();
    for path in paths {
        let Some((_, nl)) = app.sources.get(&path) else {
            continue;
        };
        if let Some(h) = highlight_file(&path, &nl.join("\n"), &app.theme.syn) {
            app.highlights.insert(path, h);
        }
    }
}

pub(super) fn config_toggle(app: &mut App) {
    let Some(c) = app.config.as_mut() else { return };
    if !c.toggle() {
        return;
    }
    let rules_changed = c.field(c.sel).is_some_and(|f| {
        f.key.starts_with("disable:") || f.key.starts_with("section:") || f.key == "catalog"
    });
    // where docs sort is the engine's decision too, so changing it means
    // asking the engine again — the same re-run a catalog switch needs
    let docs_last = match c.field(c.sel) {
        Some(f) if f.key == "docs_last" => match f.kind {
            FieldKind::Flag(v) => Some(v),
            _ => None,
        },
        _ => None,
    };
    let (catalog, disables) = config_rule_state(c);
    let (_, preset) = config_general(c);
    if let Some(v) = docs_last {
        app.docs_last = v;
    }
    if rules_changed || docs_last.is_some() {
        app.catalog = catalog;
        app.disables = disables;
        let _ = run_strategy(app, app.strategy.clone().as_str());
    }
    config_apply_theme(app);
    // guarded: a preset swap rebuilds the whole map, and doing that because a
    // catalog checkbox moved would drop the reviewer's own `[binds]`
    if let Some(k) = keymap(&preset).filter(|k| k.name != app.keys.name) {
        app.keys = k;
    }
}

/// Accept the row being typed into, and repaint if it was a colour.
pub(super) fn config_commit_edit(app: &mut App) {
    if let Some(c) = app.config.as_mut() {
        c.commit_edit();
    }
    config_apply_theme(app);
}

/// The palette the config screen currently describes — the chosen theme with
/// the filled-in role rows on top — applied if it isn't what's on screen.
/// Both a theme swap and a single edited colour arrive here.
pub(super) fn config_apply_theme(app: &mut App) {
    let Some(c) = app.config.as_ref() else { return };
    let (name, _) = config_general(c);
    let colors: Vec<(String, Color)> = c
        .sections
        .iter()
        .flat_map(|s| &s.fields)
        .filter_map(|f| {
            let role = f.key.strip_prefix("theme.")?;
            let FieldKind::Text(t) = &f.kind else {
                return None;
            };
            Some((role.to_string(), parse_hex(t)?))
        })
        .collect();
    let Some(base) = theme(&name) else { return };
    let next = apply_theme_colors(base, &colors);
    // re-highlighting every open file is not free: only a real difference
    if next.name != app.theme.name
        || THEME_ROLES
            .iter()
            .any(|r| theme_role_color(&next, r) != theme_role_color(&app.theme, r))
    {
        swap_theme(app, next);
    }
}

/// The catalog switch and the `disable` list the current UI state implies.
pub(super) fn config_rule_state(c: &ConfigUi) -> (bool, Vec<String>) {
    let mut catalog = true;
    let mut disables = vec![];
    for f in c.sections.iter().flat_map(|s| &s.fields) {
        match (&f.key, &f.kind) {
            (k, FieldKind::Flag(v)) if k == "catalog" => catalog = *v,
            (k, FieldKind::Flag(false)) => {
                if let Some(rule) = k.strip_prefix("disable:") {
                    disables.push(rule.to_string());
                }
            }
            _ => {}
        }
    }
    (catalog, disables)
}

pub(super) fn config_write(app: &mut App) {
    let Some(c) = app.config.as_ref() else { return };
    let changed = c.changed();
    if changed.is_empty() {
        return;
    }
    let mut wrote: Vec<String> = vec![];
    let mut failed: Vec<String> = vec![];
    let (settings, rules): (Vec<&ConfigField>, Vec<&ConfigField>) =
        changed.iter().partition(|f| !touches_rules(&f.key));
    if !settings.is_empty() {
        if let Some(path) = config_path() {
            match write_settings_file(&path, &settings) {
                Ok(()) => wrote.push(path.display().to_string()),
                Err(e) => failed.push(format!("{}: {e}", path.display())),
            }
        }
    }
    if !rules.is_empty() {
        if let Some(path) = user_rules_path() {
            match write_rules_file(&path, c, &rules) {
                Ok(()) => wrote.push(path.display().to_string()),
                Err(e) => failed.push(format!("{}: {e}", path.display())),
            }
        }
    }
    // only a clean write clears the dirty mark: telling a reviewer their
    // settings are saved when the disk refused is the one thing this must
    // never do
    if failed.is_empty() {
        if let Some(c) = app.config.as_mut() {
            c.original = c.sections.clone();
            c.dirty = false;
        }
    }
    let (title, lines) = if failed.is_empty() {
        ("config written", wrote)
    } else {
        ("config NOT written", failed)
    };
    app.popup = Some(Popup::new(
        title,
        lines.into_iter().map(Line::from).collect(),
    ));
}

/// a setting that lives in the rules file rather than `tui.toml`
fn touches_rules(key: &str) -> bool {
    key == "catalog"
        || key.starts_with("disable:")
        || key.starts_with("section:")
        || key.starts_with("include:")
}

/// The file's current text, or empty for a file that is not there yet. An
/// existing file that cannot be read must not be rewritten from nothing: that
/// would drop every setting already in it.
fn existing_text(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// a changed field as the (section, key, value) `tui.toml` spells it
fn toml_setting(f: &ConfigField) -> (Option<&'static str>, String, String) {
    let (section, key) = match f.key.split_once('.') {
        Some(("theme", k)) => (Some("theme"), k.to_string()),
        Some(("binds", k)) => (Some("binds"), format!("\"{k}\"")),
        _ => (None, f.key.clone()),
    };
    let value = match &f.kind {
        FieldKind::Flag(v) => v.to_string(),
        FieldKind::Choice { at, options } => {
            format!("\"{}\"", options.get(*at).cloned().unwrap_or_default())
        }
        FieldKind::Text(t) => format!("\"{t}\""),
    };
    (section, key, value)
}

fn write_settings_file(path: &Path, settings: &[&ConfigField]) -> Result<(), String> {
    let mut text = existing_text(path)?;
    for f in settings {
        let (section, key, value) = toml_setting(f);
        text = upsert_toml(&text, section, &key, &value);
    }
    write_atomic(path, &text).map_err(|e| e.to_string())
}

// each key only if something in its category moved: rewriting `include`
// because a catalog rule changed would put a line in the file that the
// reviewer never asked for
fn write_rules_file(path: &Path, c: &ConfigUi, changed: &[&ConfigField]) -> Result<(), String> {
    let mut text = existing_text(path)?;
    let touched = |p: &str| changed.iter().any(|f| f.key.starts_with(p));
    let (catalog, disables) = config_rule_state(c);
    if changed.iter().any(|f| f.key == "catalog") {
        text = upsert_toml(&text, None, "catalog", &catalog.to_string());
    }
    if touched("disable:") || touched("section:") {
        text = upsert_toml(&text, None, "disable", &toml_list(&disables));
    }
    if touched("include:") {
        text = upsert_toml(&text, None, "include", &toml_list(&config_includes(c)));
    }
    write_atomic(path, &text).map_err(|e| e.to_string())
}

/// A TOML array of strings, on one line.
pub(super) fn toml_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|i| format!("\"{i}\"")).collect();
    format!("[{}]", inner.join(", "))
}

/// The bundled rulesets the current UI state includes.
pub(super) fn config_includes(c: &ConfigUi) -> Vec<String> {
    c.sections
        .iter()
        .flat_map(|s| &s.fields)
        .filter_map(|f| match (&f.key, &f.kind) {
            (k, FieldKind::Flag(true)) => k.strip_prefix("include:").map(str::to_string),
            _ => None,
        })
        .collect()
}

/// The preset and theme the current UI state implies.
pub(super) fn config_general(c: &ConfigUi) -> (String, String) {
    let pick = |key: &str| {
        c.sections
            .iter()
            .flat_map(|s| &s.fields)
            .find(|f| f.key == key)
            .and_then(|f| match &f.kind {
                FieldKind::Choice { at, options } => options.get(*at).cloned(),
                _ => None,
            })
            .unwrap_or_default()
    };
    (pick("theme"), pick("preset"))
}

/// Render `:config` over the panes: a section at a time, current value on the
/// right, the file each section writes to in its header.
pub(super) fn draw_config(f: &mut Frame, app: &App, body: Rect) {
    let Some(c) = app.config.as_ref() else { return };
    let theme = &app.theme;
    let area = centred(body, 96, body.height.saturating_sub(4));
    f.render_widget(Clear, area);
    let title = format!(
        " config{} — {} ",
        if c.dirty { " ·" } else { "" },
        if c.editing.is_some() {
            "typing · Enter accept · Esc cancel"
        } else {
            "Space change · w write · Esc close"
        }
    );
    // the selected setting's own sentence, on the bottom border: a list of
    // names is not a config UI if nothing says what they do
    let help = c
        .field(c.sel)
        .map(|f| f.help.clone())
        .filter(|h| !h.is_empty())
        .unwrap_or_default();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(Span::styled(title, Style::default().fg(theme.border_focus)))
        .title_bottom(Span::styled(
            if help.is_empty() {
                String::new()
            } else {
                format!(" {help} ")
            },
            Style::default().fg(theme.dim),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = inner.width as usize;
    let mut lines: Vec<Line<'static>> = vec![];
    let mut row_of_sel = 0usize;
    for (i, sec) in c.sections.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", sec.title),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("→ {}", sec.file), Style::default().fg(theme.dim)),
        ]));
        for (j, field) in sec.fields.iter().enumerate() {
            let at = c.rows.iter().position(|&r| r == (i, j)).unwrap_or(0);
            if at == c.sel {
                row_of_sel = lines.len();
            }
            let value = match &field.kind {
                FieldKind::Flag(true) => "on".to_string(),
                FieldKind::Flag(false) => "off".to_string(),
                FieldKind::Choice { at, options } => options.get(*at).cloned().unwrap_or_default(),
                // an empty text field is not the same as one that follows the
                // terminal; say which
                FieldKind::Text(t) if t.is_empty() && field.key.starts_with("theme.") => {
                    "(terminal)".to_string()
                }
                FieldKind::Text(t) if t.is_empty() => "(unset)".to_string(),
                FieldKind::Text(t) => t.clone(),
            };
            let editing = at == c.sel && c.editing.is_some();
            let value = match &c.editing {
                Some(buf) if editing => format!("{buf}▏"),
                _ => value,
            };
            let off = matches!(field.kind, FieldKind::Flag(false));
            let pad = width.saturating_sub(field.label.chars().count() + value.chars().count() + 3);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}{} ", if at == c.sel { "▸ " } else { "  " }, field.label),
                    Style::default().fg(if at == c.sel {
                        theme.border_focus
                    } else {
                        theme.fg
                    }),
                ),
                Span::styled(" ".repeat(pad), Style::default()),
                Span::styled(
                    value,
                    Style::default().fg(if off { theme.dim } else { theme.accent }),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }
    // keep the selected row on screen without a scrollbar to maintain
    let h = inner.height.max(1) as usize;
    let top = row_of_sel
        .saturating_sub(h / 2)
        .min(lines.len().saturating_sub(h));
    f.render_widget(
        Paragraph::new(Text::from(lines)).scroll((top as u16, 0)),
        inner,
    );
    let _ = c.scroll;
}
