//! The lists of the settings as tables: the アカウント, データベース, 接続先 PC, S3 and メールボックス pages, the 利用者
//! page and the box that manages the `[[remote]]`s. One entry is one row; the last column holds the row's buttons.
//!
//! This file draws the table on [`TodayTable`] (the run page's table) and keeps what a table remembers
//! ([`Memory`]: the sort and the picked row). What a button does stays with each page: the table only says which
//! action of which row was meant ([`show`] gives `(row, action key)`), whether it came from the button, from a
//! double-click on the row (the action that edits, when it is enabled) or from the right-click menu (the enabled
//! actions). The page turns that into the same click it has always had, so every confirmation still stands.
//!
//! The pieces that need no window are plain functions ([`order`], [`Memory::sync`], [`row_words`]) and are
//! tested as such.

use std::hash::{Hash, Hasher};

use ito_a11y::Role;
use ito_prefs::{IconButton, Mark, Tone};

use super::users::tid;
use super::TodayTable;

/// One cell: its words (one line; parts are joined with 「・」), the words that show on hover instead of its own when it
/// has some, and whether it is drawn in the warning colour (a secret that is missing or cannot be read).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Cell {
    pub text: String,
    pub hover: Option<String>,
    pub warn: bool,
    /// What the screen reader says of the cell when it is more than its text.
    pub said: Option<String>,
}

impl Cell {
    pub(super) fn new(text: impl Into<String>) -> Self {
        Cell { text: text.into(), hover: None, warn: false, said: None }
    }

    pub(super) fn warn(mut self, warn: bool) -> Self {
        self.warn = warn;
        self
    }

    pub(super) fn said(mut self, words: impl Into<String>) -> Self {
        self.said = Some(words.into());
        self
    }

    pub(super) fn hover(mut self, words: impl Into<String>) -> Self {
        self.hover = Some(words.into());
        self
    }
}

/// One thing a row can do: its button, and its place in the double-click and the right-click menu.
#[derive(Clone)]
pub(super) struct Act {
    /// The action's name in a test ID (`<prefix>.menu.<key>.<id>`) and in what [`show`] gives back.
    pub key: &'static str,
    pub words: String,
    /// The spoken name of the button, when it differs from its words.
    pub said: Option<String>,
    pub mark: Mark,
    pub tone: Tone,
    /// The button is on now. An action that is off is left out of the menu.
    pub on: bool,
    /// Why it is off (the button's hover).
    pub why: Option<String>,
    /// The button's test ID.
    pub id: Option<String>,
    /// A double-click on the row does this (when it is on).
    pub edit: bool,
}

impl Act {
    pub(super) fn new(key: &'static str, words: impl Into<String>, mark: Mark, tone: Tone) -> Self {
        Act { key, words: words.into(), said: None, mark, tone, on: true, why: None, id: None, edit: false }
    }

    pub(super) fn on(mut self, on: bool, why: Option<&str>) -> Self {
        self.on = on;
        self.why = why.map(str::to_owned);
        self
    }

    pub(super) fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub(super) fn said(mut self, said: impl Into<String>) -> Self {
        self.said = Some(said.into());
        self
    }

    pub(super) fn edit(mut self) -> Self {
        self.edit = true;
        self
    }
}

/// One row: the entry's name (its pick, and the `<id>` of its test IDs), the data cells and the actions.
#[derive(Clone)]
pub(super) struct Row {
    pub name: String,
    pub cells: Vec<Cell>,
    pub acts: Vec<Act>,
    /// The width of what the page draws itself in the buttons' column ([`show`]'s `extra`), with its spacing.
    pub extra_w: f32,
}

impl Row {
    pub(super) fn new(name: impl Into<String>, cells: Vec<Cell>, acts: Vec<Act>) -> Self {
        Row { name: name.into(), cells, acts, extra_w: 0.0 }
    }

    /// What the screen reader says of the row: `<head>: <cell>, …` (the empty cells are left out).
    pub(super) fn words(&self, heads: &[String]) -> String {
        row_words(heads, &self.cells)
    }
}

/// `<head>: <cell>, …` of a row; a cell that is empty says nothing.
pub(super) fn row_words(heads: &[String], cells: &[Cell]) -> String {
    let said = |c: &Cell| c.said.clone().unwrap_or_else(|| c.text.clone());
    heads.iter().zip(cells).filter(|(_, c)| !c.text.is_empty()).map(|(h, c)| format!("{h}: {}", said(c))).collect::<Vec<_>>().join(", ")
}

/// What one table keeps: the sort (▲ → ▼ → none; in memory only) and the picked row.
#[derive(Default)]
pub(super) struct Memory {
    sort: Option<(usize, bool)>,
    /// The engine linked to when the row was picked, and the entry's name.
    pick: Option<(Option<String>, String)>,
    /// The form that was scrolled into view, so it is scrolled once.
    scrolled: Option<u64>,
}

impl Memory {
    #[cfg(test)]
    pub(super) fn sort(&self) -> Option<(usize, bool)> {
        self.sort
    }

    /// A click on column `col`'s head.
    pub(super) fn sort_by(&mut self, col: usize) {
        self.sort = TodayTable::next_sort(self.sort, col);
    }

    #[cfg(test)]
    pub(super) fn picked(&self) -> Option<&str> {
        self.pick.as_ref().map(|(_, n)| n.as_str())
    }

    pub(super) fn is_picked(&self, link: Option<&str>, name: &str) -> bool {
        self.pick.as_ref().is_some_and(|(l, n)| l.as_deref() == link && n == name)
    }

    pub(super) fn pick(&mut self, link: Option<&str>, name: &str) {
        self.pick = Some((link.map(str::to_owned), name.to_owned()));
    }

    /// Drops a pick that no longer stands: the engine linked to changed, or the list is LOADED (`Some`: no reload
    /// is pending) and the entry is not in it. While the list is being loaded (`None`) the pick is kept.
    pub(super) fn sync(&mut self, link: Option<&str>, loaded: Option<&[String]>) {
        let Some((l, n)) = &self.pick else { return };
        if l.as_deref() != link || loaded.is_some_and(|names| !names.iter().any(|x| x == n)) {
            self.pick = None;
        }
    }

    /// Scrolls the page to `rect` once for each form: `token` names the form that is open now (`None`: none).
    fn scroll_once(&mut self, ui: &egui::Ui, rect: egui::Rect, token: Option<u64>) {
        if token != self.scrolled {
            if token.is_some() {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
            self.scrolled = token;
        }
    }
}

/// A number for "which form is this" (its kind and the entry's name), to scroll to it once.
pub(super) fn form_token(kind: &str, name: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind, name).hash(&mut h);
    h.finish()
}

/// The order the rows are shown in: with no sort the order they came in; with one, the words of that column
/// compared naturally and without case (`job2` before `job10`), the empty ones last either way.
pub(super) fn order(rows: &[Row], sort: Option<(usize, bool)>) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    let Some((col, down)) = sort else { return idx };
    let text = |i: usize| rows[i].cells.get(col).map_or("", |c| c.text.as_str());
    idx.sort_by(|&a, &b| {
        let (x, y) = (text(a), text(b));
        match (x.is_empty(), y.is_empty()) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (true, true) => std::cmp::Ordering::Equal,
            _ => {
                let o = ito_common::text::natural_cmp(x, y, false);
                if down {
                    o.reverse()
                } else {
                    o
                }
            }
        }
    });
    idx
}

/// What a table is called by: its test ID prefix, its heads (the data columns, then the buttons' column), the hover of
/// a sortable head, and the engine linked to (the pick belongs to it).
pub(super) struct Spec<'a> {
    pub prefix: &'a str,
    pub heads: &'a [String],
    pub hint: &'a str,
    pub link: Option<&'a str>,
}

/// The width of a row's buttons, with their spacing, and `extra_w`.
fn ops_width(ui: &egui::Ui, row: &Row) -> f32 {
    let gap = ui.spacing().item_spacing.x;
    let mut w = row.extra_w;
    for a in &row.acts {
        w += IconButton::new(a.mark, &a.words).size(ui).x + gap;
    }
    (w - if row.extra_w > 0.0 || row.acts.is_empty() { 0.0 } else { gap }).max(0.0)
}

/// One of a row's buttons; true when it was pressed.
pub(super) fn button(ui: &mut egui::Ui, a: &Act) -> bool {
    let mut b = IconButton::new(a.mark, &a.words).tone(a.tone);
    if let Some(said) = &a.said {
        b = b.said(said);
    }
    let r = ui.add_enabled_ui(a.on, |ui| b.show(ui)).inner;
    if let Some(id) = &a.id {
        ito_a11y::test_id(&r, id);
    }
    if !a.on {
        if let Some(why) = &a.why {
            r.on_disabled_hover_text(why);
        }
        return false;
    }
    r.clicked()
}

/// What a page draws in the 操作 cell of a row (given its index), before the buttons.
pub(super) type Extra<'a> = dyn FnMut(&mut egui::Ui, usize) + 'a;

/// Draws the table of `rows` and gives the action that was meant, as `(index in rows, the action's key)`.
///
/// Columns fit their words and the words give way (ending in `…`) to the visible width, which is read here, outside
/// the scroll area; what still does not fit scrolls sideways. A click on a data cell picks the row, a double-click does
/// what the row's `edit` action does (when it is on), a right-click picks it and opens a menu of the actions that are on.
/// `extra` draws more in the row's buttons' column (before the buttons), for the row at that index.
pub(super) fn show(
    ui: &mut egui::Ui,
    mem: &mut Memory,
    spec: &Spec,
    rows: &[Row],
    mut extra: Option<&mut Extra>,
) -> Option<(usize, &'static str)> {
    let c = ito_theme::colors();
    let visible = ui.available_width();
    let last = spec.heads.len().saturating_sub(1);
    let shown = order(rows, mem.sort);
    let mut out = None;
    let mut sort_hit = None;
    egui::ScrollArea::horizontal().id_salt((spec.prefix, "scroll")).auto_shrink([false, true]).show(ui, |ui| {
        let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
        let grid: Vec<Vec<(String, bool)>> = rows
            .iter()
            .map(|r| {
                let mut cells: Vec<(String, bool)> = r.cells.iter().map(|c| (c.text.clone(), false)).collect();
                cells.push((String::new(), false));
                cells
            })
            .collect();
        let mut widths = TodayTable::fit(ui, spec.heads, &grid);
        let ops = rows.iter().map(|r| ops_width(ui, r)).fold(0.0, f32::max);
        if let Some(w) = widths.get_mut(last) {
            *w = w.max(ops + TodayTable::PAD + 8.0);
        }
        let flex: Vec<usize> = (0..last).collect();
        TodayTable::squeeze(&mut widths, visible, &flex, 72.0);
        let mut table = TodayTable::with(&c, &widths, visible);
        sort_hit = table.head_to(ui, spec.heads, mem.sort, spec.hint, last);
        let ops_h = IconButton::new(Mark::Pencil, "").size(ui).y;
        for &i in &shown {
            let row = &rows[i];
            let id = tid(&row.name);
            let cells: Vec<super::ListCell> = row
                .cells
                .iter()
                .map(|cell| super::ListCell { text: &cell.text, ink: cell.warn.then_some(c.wait), hover: cell.hover.as_deref() })
                .collect();
            let on = mem.is_picked(spec.link, &row.name);
            let rid = ui.id().with((spec.prefix, "row", &row.name));
            let (r, ops_rect) = table.list_row(ui, rid, &cells, on, ops_h);
            ito_a11y::say(&r, Role::Row, &row.words(spec.heads));
            ito_a11y::test_id(&r, &format!("{}.row.{id}", spec.prefix));
            if r.clicked() || r.secondary_clicked() {
                mem.pick(spec.link, &row.name);
            }
            if r.double_clicked() {
                if let Some(a) = row.acts.iter().find(|a| a.edit && a.on) {
                    out = Some((i, a.key));
                }
            }
            let menu = ui.make_persistent_id((spec.prefix, "menu", &row.name));
            if row.acts.iter().any(|a| a.on) {
                egui::Popup::context_menu(&r).id(menu).show(|ui| {
                    for a in row.acts.iter().filter(|a| a.on) {
                        let b = ui.add(egui::Button::new(&a.words));
                        ito_a11y::say(&b, Role::Button, &a.words);
                        ito_a11y::test_id(&b, &format!("{}.menu.{}.{id}", spec.prefix, a.key));
                        if b.clicked() {
                            out = Some((i, a.key));
                            ui.close();
                        }
                    }
                });
            }
            let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(ops_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
            if let Some(draw) = extra.as_deref_mut() {
                draw(&mut cell, i);
            }
            for a in &row.acts {
                if button(&mut cell, a) {
                    out = Some((i, a.key));
                }
            }
        }
        ui.spacing_mut().item_spacing.y = gap;
    });
    if let Some(col) = sort_hit {
        mem.sort_by(col);
    }
    out
}

/// The form of a row, below the table (outside the sideways scroll), at the visible width: framed, with the entry's
/// name as its heading, and scrolled into view once when it opens. `token` names the form ([`form_token`]).
pub(super) fn form_below<R>(ui: &mut egui::Ui, mem: &mut Memory, heading: &str, token: u64, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let framed = egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let r = ui.label(egui::RichText::new(heading).strong());
        ito_a11y::say(&r, Role::Heading, heading);
        add(ui)
    });
    mem.scroll_once(ui, framed.response.rect, Some(token));
    framed.inner
}

/// No form is open for this table: the next one scrolls into view again.
pub(super) fn form_closed(mem: &mut Memory) {
    mem.scrolled = None;
}

/// A harness for the tests of the pages: a context with the screen reader on, frames with a pointer, and the places
/// of the words the screen reader was told.
#[cfg(test)]
pub(super) mod pad {
    use std::collections::BTreeMap;

    pub(in crate::ui) struct Pad {
        pub ctx: egui::Context,
        pub size: egui::Vec2,
        pub nodes: BTreeMap<u64, (String, egui::Rect)>,
        /// The clock of the frames: a double-click is two presses close in time, and a later one is a new click.
        pub time: f64,
    }

    impl Pad {
        pub fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            Pad { ctx, size: egui::vec2(1280.0, 800.0), nodes: BTreeMap::new(), time: 0.0 }
        }

        pub fn frame(&mut self, events: Vec<egui::Event>, f: &mut dyn FnMut(&mut egui::Ui)) {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
            self.time += 0.05;
            let input = egui::RawInput { screen_rect: Some(rect), events, time: Some(self.time), ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| f(ui));
            out.textures_delta.clear();
            if let Some(update) = out.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    let place = node
                        .bounds()
                        .map(|r| egui::Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32)));
                    match (node.label().or_else(|| node.value()), place) {
                        (Some(label), Some(place)) => drop(self.nodes.insert(id.0, (label.to_owned(), place))),
                        _ => drop(self.nodes.remove(&id.0)),
                    }
                }
            }
        }

        pub fn settle(&mut self, f: &mut dyn FnMut(&mut egui::Ui)) {
            for _ in 0..3 {
                self.frame(Vec::new(), f);
            }
            self.time += 1.0;
        }

        /// Where the one widget whose words are `words`, or else contain them, is.
        pub fn rect(&self, words: &str) -> egui::Rect {
            let exact: Vec<_> = self.nodes.values().filter(|(label, _)| label == words).collect();
            let found: Vec<_> =
                if exact.is_empty() { self.nodes.values().filter(|(label, _)| label.contains(words)).collect() } else { exact };
            let mut one: Vec<&(String, egui::Rect)> = Vec::new();
            for f in found {
                if !one.contains(&f) {
                    one.push(f);
                }
            }
            assert_eq!(one.len(), 1, "one widget says {words:?}: {:?}", self.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
            one[0].1
        }

        /// The topmost widget that says exactly `words` (a head, where the form below has the same word).
        pub fn topmost(&self, words: &str) -> egui::Rect {
            let found = self.nodes.values().filter(|(l, _)| l == words).map(|(_, r)| *r);
            found.min_by(|a, b| a.top().total_cmp(&b.top())).unwrap_or_else(|| panic!("no widget says {words:?}"))
        }

        /// Where the widget that says exactly `words` is, on the row that says `row`.
        pub fn in_row(&self, row: &str, words: &str) -> egui::Rect {
            let r = self.rect(row);
            let found: Vec<_> =
                self.nodes.values().filter(|(l, p)| l == words && (p.center().y - r.center().y).abs() < r.height() / 2.0).collect();
            assert_eq!(found.len(), 1, "one {words:?} on the row {row:?}: {:?}", self.nodes.values().map(|n| &n.0).collect::<Vec<_>>());
            found[0].1
        }

        pub fn presses_at(&mut self, pos: egui::Pos2, button: egui::PointerButton, times: usize, f: &mut dyn FnMut(&mut egui::Ui)) {
            let ev = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
            self.frame(vec![egui::Event::PointerMoved(pos)], f);
            for _ in 0..times {
                self.frame(vec![ev(true)], f);
                self.frame(vec![ev(false)], f);
            }
            self.settle(f);
        }

        pub fn right_click(&mut self, words: &str, f: &mut dyn FnMut(&mut egui::Ui)) {
            let pos = self.rect(words).center();
            self.presses_at(pos, egui::PointerButton::Secondary, 1, f);
        }

        /// A press in the data area of the row that says `words`: at the left of its first cell.
        pub fn click_row(&mut self, words: &str, button: egui::PointerButton, times: usize, f: &mut dyn FnMut(&mut egui::Ui)) {
            let r = self.rect(words);
            self.presses_at(egui::pos2(r.left() + 14.0, r.center().y), button, times, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pad::Pad;
    use super::*;

    fn row(name: &str, second: &str) -> Row {
        Row::new(
            name,
            vec![Cell::new(name), Cell::new(second)],
            vec![
                Act::new("edit", "編集…", Mark::Pencil, Tone::Plain).id(format!("t.edit.{}", tid(name))).edit(),
                Act::new("remove", "削除…", Mark::Trash, Tone::Danger).id(format!("t.remove.{}", tid(name))),
            ],
        )
    }

    fn heads() -> Vec<String> {
        ["名前", "種類", "操作"].map(str::to_owned).to_vec()
    }

    #[test]
    fn rows_sort_naturally_with_the_empty_last_both_ways() {
        let rows = vec![row("job10", "b"), row("job2", ""), row("Job1", "a")];
        assert_eq!(order(&rows, None), vec![0, 1, 2], "no sort keeps the order");
        assert_eq!(order(&rows, Some((0, false))), vec![2, 1, 0], "job2 before job10, case apart");
        assert_eq!(order(&rows, Some((0, true))), vec![0, 1, 2]);
        assert_eq!(order(&rows, Some((1, false))), vec![2, 0, 1], "the empty one last");
        assert_eq!(order(&rows, Some((1, true))), vec![0, 2, 1], "and last when going down too");
    }

    #[test]
    fn the_head_cycles_up_down_none() {
        let mut m = Memory::default();
        m.sort_by(1);
        assert_eq!(m.sort(), Some((1, false)));
        m.sort_by(1);
        assert_eq!(m.sort(), Some((1, true)));
        m.sort_by(1);
        assert_eq!(m.sort(), None);
        m.sort_by(0);
        m.sort_by(2);
        assert_eq!(m.sort(), Some((2, false)), "another column starts up");
    }

    #[test]
    fn the_pick_is_kept_by_name_and_dropped_when_the_list_or_the_link_lacks_it() {
        let names = vec!["a".to_owned(), "b".to_owned()];
        let mut m = Memory::default();
        m.pick(None, "b");
        assert!(m.is_picked(None, "b") && !m.is_picked(Some("pc"), "b") && !m.is_picked(None, "a"));
        m.sync(None, Some(&names));
        assert_eq!(m.picked(), Some("b"));
        // A reload is pending: the list is not known, the pick stays.
        m.sync(None, None);
        assert_eq!(m.picked(), Some("b"));
        // The loaded list lacks it.
        m.sync(None, Some(&names[..1]));
        assert_eq!(m.picked(), None);
        // Another engine drops it.
        m.pick(None, "a");
        m.sync(Some("pc"), Some(&names));
        assert_eq!(m.picked(), None);
    }

    #[test]
    fn a_row_says_its_head_and_cell_and_skips_the_empty() {
        let r = Row::new("a", vec![Cell::new("a"), Cell::new(""), Cell::new("x")], vec![]);
        let heads: Vec<String> = ["名前", "種類", "場所", "操作"].map(str::to_owned).to_vec();
        assert_eq!(r.words(&heads), "名前: a, 場所: x");
    }

    /// One frame of a table of two rows; what the page got back is in `got`.
    fn page<'a>(mem: &'a mut Memory, got: &'a mut Vec<(usize, &'static str)>, rows: &'a [Row]) -> impl FnMut(&mut egui::Ui) + 'a {
        move |ui| {
            let heads = heads();
            mem.sync(None, Some(&rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>()));
            let spec = Spec { prefix: "t", heads: &heads, hint: "sort", link: None };
            if let Some(hit) = show(ui, mem, &spec, rows, None) {
                got.push(hit);
            }
        }
    }

    /// A row with more cells than heads, and no heads at all, draw without a panic.
    #[test]
    fn a_row_with_more_cells_than_heads_or_no_heads_does_not_panic() {
        let mut wide = row("alpha", "one");
        wide.cells.push(Cell::new("three"));
        let rows = vec![wide];
        let ctx = egui::Context::default();
        for heads in [Vec::new(), vec!["名前".to_owned()], heads()] {
            let mut mem = Memory::default();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let spec = Spec { prefix: "t", heads: &heads, hint: "", link: None };
                show(ui, &mut mem, &spec, &rows, None);
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn a_button_acts_and_a_click_on_the_row_only_picks() {
        let rows = vec![row("alpha", "one"), row("beta", "two")];
        let (mut mem, mut got) = (Memory::default(), Vec::new());
        let mut pad = Pad::new();
        pad.settle(&mut page(&mut mem, &mut got, &rows));
        pad.click_row("名前: beta", egui::PointerButton::Primary, 1, &mut page(&mut mem, &mut got, &rows));
        assert_eq!(mem.picked(), Some("beta"));
        assert!(got.is_empty(), "a click on the data picks and does nothing else");
        // The buttons of the second row: one per row has the same words, so go by place.
        let rect = pad.rect("名前: beta");
        let removes: Vec<_> = pad.nodes.values().filter(|(l, _)| l == "削除…").map(|(_, r)| *r).collect();
        let button = removes.iter().find(|r| (r.center().y - rect.center().y).abs() < 4.0).expect("a 削除… on the row");
        pad.presses_at(button.center(), egui::PointerButton::Primary, 1, &mut page(&mut mem, &mut got, &rows));
        assert_eq!(got, vec![(1, "remove")], "a button gives its action for its row");
    }

    #[test]
    fn a_double_click_edits_only_when_the_edit_is_on() {
        let mut rows = vec![row("alpha", "one"), row("beta", "two")];
        rows[1].acts[0] = rows[1].acts[0].clone().on(false, Some("no"));
        let (mut mem, mut got) = (Memory::default(), Vec::new());
        let mut pad = Pad::new();
        pad.settle(&mut page(&mut mem, &mut got, &rows));
        pad.click_row("名前: alpha", egui::PointerButton::Primary, 2, &mut page(&mut mem, &mut got, &rows));
        assert_eq!(got, vec![(0, "edit")]);
        got.clear();
        pad.click_row("名前: beta", egui::PointerButton::Primary, 2, &mut page(&mut mem, &mut got, &rows));
        assert!(got.is_empty(), "an edit that is off opens nothing");
        assert_eq!(mem.picked(), Some("beta"));
    }

    #[test]
    fn the_menu_lists_only_the_actions_that_are_on() {
        let mut rows = vec![row("alpha", "one"), row("beta", "two")];
        rows[1].acts[0] = rows[1].acts[0].clone().on(false, Some("no"));
        let (mut mem, mut got) = (Memory::default(), Vec::new());
        let mut pad = Pad::new();
        pad.settle(&mut page(&mut mem, &mut got, &rows));
        pad.right_click("名前: beta", &mut page(&mut mem, &mut got, &rows));
        assert_eq!(mem.picked(), Some("beta"), "a right-click picks the row");
        let menu: Vec<_> = pad.nodes.values().filter(|(_, r)| r.center().x > 0.0).filter(|(l, _)| l == "削除…" || l == "編集…").collect();
        // Two 削除… (the button of each row) and one more in the menu; no 編集… in the menu for beta.
        assert_eq!(menu.iter().filter(|(l, _)| l == "削除…").count(), 3, "{menu:?}");
        assert_eq!(menu.iter().filter(|(l, _)| l == "編集…").count(), 2, "alpha's button and beta's (off) button only: {menu:?}");
    }
}
