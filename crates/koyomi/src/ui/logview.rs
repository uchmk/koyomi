//! One log viewer for every place that shows a job's log (the run page's ログ tab, the wide view, a job's property
//! window): find bar, highlight, next / previous hit, "only matching lines", wrap, copy and save.
//!
//! One search result drives all of it: the highlight, the bar's count, next / previous, the matching lines and what
//! コピー copies. The search runs on a worker thread (the log can be 64 KiB or more), keyed by `(gen, bar version)`.
//!
//! The caller keeps one [`LogView`] per place and calls [`LogView::show`] each frame with the text and a number
//! (`gen`) that changes whenever the text changes.

use std::borrow::Cow;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use egui::text::{LayoutJob, LayoutSection};
use ito_a11y::Role;
use ito_find::{FindBar, Status};
use ito_match::{Query, Subst};

/// One hit: the line (as `str::lines` counts) and the byte range within that line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub line: usize,
    pub range: Range<usize>,
}

/// Every hit of a query in a text, ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub hits: Vec<Hit>,
}

impl Found {
    /// The lines that have a hit, ascending, each once.
    fn lines(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.hits.iter().map(|h| h.line).collect();
        v.dedup();
        v
    }
}

/// The hits of `find` (read as `q`) in `text`, line by line. One `Subst` is compiled for the whole search.
///
/// A hit never spans two lines, and an empty (zero-width) match is not a hit: a line counts as matching only when it
/// has at least one non-empty hit. (`show::filter_log` uses `Matcher::is_match`, which also counts an empty match; the
/// viewer does not, so the count, the highlight and the lines shown always agree.)
pub fn search(text: &str, find: &str, q: Query) -> Result<Found, String> {
    if find.is_empty() {
        return Ok(Found::default());
    }
    let s = Subst::from_query(find, "", q)?;
    let mut hits = Vec::new();
    for (line, l) in text.lines().enumerate() {
        for (range, _) in s.edits(l) {
            if range.start < range.end {
                hits.push(Hit { line, range });
            }
        }
    }
    Ok(Found { hits })
}

/// The hit after / before `current`, wrapping around. `total == 0` gives 0.
pub fn step(current: usize, total: usize, forward: bool) -> usize {
    if total == 0 {
        0
    } else if forward {
        (current + 1) % total
    } else {
        (current + total - 1) % total
    }
}

/// The current hit after a new result: kept while it is still in range, else the first.
pub fn keep(current: usize, total: usize) -> usize {
    if current < total {
        current
    } else {
        0
    }
}

/// What is shown (and what コピー copies) and the hits in it, with the lines as numbered in that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View<'a> {
    pub text: Cow<'a, str>,
    pub hits: Cow<'a, [Hit]>,
}

/// The view of `text` for the search result `res` (`None`: no valid search yet, or an empty query). A bad query or no
/// result gives the whole text with no hits (and `only_hits` is ignored); a good one with `only_hits` gives only the
/// matching lines, joined with `\n`, with the hits renumbered.
pub fn view<'a>(text: &'a str, res: Option<&'a Result<Found, String>>, only_hits: bool) -> View<'a> {
    match res {
        Some(Ok(f)) if only_hits => {
            let lines = f.lines();
            let mut at = 0;
            let mut shown = Vec::new();
            for (i, l) in text.lines().enumerate() {
                if lines.get(at) == Some(&i) {
                    shown.push(l);
                    at += 1;
                }
            }
            let hits = f.hits.iter().map(|h| Hit { line: lines.binary_search(&h.line).unwrap_or(0), range: h.range.clone() }).collect();
            View { text: Cow::Owned(shown.join("\n")), hits: Cow::Owned(hits) }
        }
        Some(Ok(f)) => View { text: Cow::Borrowed(text), hits: Cow::Borrowed(&f.hits) },
        _ => View { text: Cow::Borrowed(text), hits: Cow::Borrowed(&[]) },
    }
}

/// The line (of the shown text) to bring into view for hit `current`.
pub fn scroll_line(hits: &[Hit], current: usize) -> Option<usize> {
    hits.get(current).map(|h| h.line)
}

/// The file name to propose: the place's name (blank: `log`) with `.log`, made safe on Windows by ito-common.
pub fn save_file_name(name: &str) -> String {
    let n = if name.trim().is_empty() { "log" } else { name };
    ito_common::safe_file_name(&format!("{n}.log"))
}

/// Whether every hit lies inside `text` (a guard against a caller whose `gen` did not change with its text).
fn fits(text: &str, f: &Found) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    f.hits.iter().all(|h| {
        lines.get(h.line).is_some_and(|l| {
            h.range.start <= h.range.end && h.range.end <= l.len() && l.is_char_boundary(h.range.start) && l.is_char_boundary(h.range.end)
        })
    })
}

/// What is drawn: the shown text and its hits, made for one text number and one query version. It is kept (and
/// drawn again) while a search for a newer text of the same query is pending, so the body does not flicker, and it
/// is the cache: rebuilt only when the text number, the query version or `only_hits` changes.
struct Drawn {
    serial: u64,
    gen: u64,
    /// The query version it was made for; `None`: the plain text (no query, a bad one, or no result that fits).
    version: Option<u64>,
    only_hits: bool,
    text: String,
    hits: Vec<Hit>,
    /// The byte offset of each line of `text` (for the wrapped job).
    starts: Vec<usize>,
}

/// What the wrapped galley was laid out for.
#[derive(Clone, PartialEq)]
struct GalleyKey {
    serial: u64,
    current: usize,
    width: u32,
    ppp: u32,
    font: egui::FontId,
    colors: [egui::Color32; 4],
}

type GalleyCache = Option<(GalleyKey, Arc<egui::Galley>)>;

#[cfg(test)]
thread_local! {
    /// Counts of the rebuilds of the drawn view and of the wrapped layout, for the "idle frame does no work" test.
    static REBUILDS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

// ---- the search thread ----

/// A finished search: the text number and the query version it was for.
struct Done {
    gen: u64,
    version: u64,
    found: Result<Found, String>,
}

struct Req {
    gen: u64,
    version: u64,
    text: Arc<str>,
    find: String,
    query: Query,
    ctx: egui::Context,
    /// The viewport the search was asked from: its frame is asked for when the result is ready (a second window
    /// is not the root's pass).
    viewport: egui::ViewportId,
}

/// The one place a request waits for the worker: a newer request replaces an older one that was not taken yet.
#[derive(Default)]
struct Slot {
    req: Option<Req>,
    closed: bool,
}

impl Slot {
    fn put(&mut self, r: Req) {
        self.req = Some(r);
    }
}

#[derive(Default)]
struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}

impl Shared {
    fn put(&self, r: Req) {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner).put(r);
        self.wake.notify_one();
    }

    fn close(&self) {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner).closed = true;
        self.wake.notify_one();
    }

    /// The next request, waiting for one; `None` when closed.
    fn next(&self) -> Option<Req> {
        let mut g = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if g.closed {
                return None;
            }
            if let Some(r) = g.req.take() {
                return Some(r);
            }
            g = self.wake.wait(g).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn work(shared: Arc<Shared>, tx: Sender<Done>) {
    while let Some(r) = shared.next() {
        let found = search(&r.text, &r.find, r.query);
        if tx.send(Done { gen: r.gen, version: r.version, found }).is_err() {
            return;
        }
        r.ctx.request_repaint_of(r.viewport);
    }
}

/// The last result when it is for this text, this query and a non-empty query.
fn valid_of(done: &Option<Done>, version: u64, find_empty: bool, gen: u64) -> Option<&Result<Found, String>> {
    done.as_ref().filter(|d| d.gen == gen && d.version == version && !find_empty).map(|d| &d.found)
}

/// A result is taken only when it is for the newest key.
fn accept(want: (u64, u64), got: (u64, u64)) -> bool {
    want == got
}

struct Worker {
    shared: Arc<Shared>,
    rx: Receiver<Done>,
}

impl Worker {
    fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let s = shared.clone();
        std::thread::spawn(move || work(s, tx));
        Worker { shared, rx }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.close();
    }
}

// ---- the words, what happened, the state ----

/// Every visible word, from the caller's `lang/*.toml`.
#[derive(Clone, Copy, Debug)]
pub struct Words<'a> {
    /// 折り返す
    pub wrap: &'a str,
    /// 一致した行だけ
    pub only_hits: &'a str,
    pub copy: &'a str,
    /// 保存…
    pub save: &'a str,
    /// The hover of the save button: what is saved (the text the window holds, not the whole file).
    pub save_hint: &'a str,
    /// The text when there is no log.
    pub none: &'a str,
    pub find: ito_find::Words<'a>,
}

/// What happened this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    /// コピー was pressed: `Ok` or the clipboard's words.
    pub copied: Option<Result<(), String>>,
    /// A save finished: the path written or the OS's words (a cancelled dialog says nothing).
    pub saved: Option<Result<PathBuf, String>>,
    /// The bar's Esc (find box focused) or ✕ cleared the query.
    pub closed: bool,
}

type Saving = Receiver<Option<Result<PathBuf, String>>>;

/// The state of one log viewer; the caller keeps one per place.
pub struct LogView {
    ids: String,
    bar: FindBar,
    pub only_hits: bool,
    pub wrap: bool,
    /// The current hit (0-based), an index into the hits of the shown text.
    current: usize,
    /// The query version `current` belongs to.
    current_version: Option<u64>,
    /// The last result (the newest one taken) and the key last sent to the worker.
    done: Option<Done>,
    sent: Option<(u64, u64)>,
    worker: Option<Worker>,
    /// The hit to bring into view, once.
    scroll: bool,
    saving: Option<Saving>,
    drawn: Option<Drawn>,
    serial: u64,
    galley: GalleyCache,
}

impl LogView {
    /// `ids` is the place's test-ID prefix (`run.log`, `run.wide`, `props.log`); the bar gets `<ids>.find`.
    pub fn new(ids: &str) -> Self {
        let mut bar = FindBar::read_only().ids(format!("{ids}.find"));
        bar.open = true;
        LogView {
            ids: ids.to_owned(),
            bar,
            only_hits: false,
            wrap: true,
            current: 0,
            current_version: None,
            done: None,
            sent: None,
            worker: None,
            scroll: false,
            saving: None,
            drawn: None,
            serial: 0,
            galley: None,
        }
    }

    fn reset_search(&mut self) {
        self.done = None;
        self.sent = None;
        self.current = 0;
        self.current_version = None;
        self.scroll = false;
        self.bar.status = Status::Idle;
    }

    /// The words in the find box.
    #[cfg(test)]
    pub fn find_text(&self) -> &str {
        &self.bar.find
    }

    /// Takes the search of `from` (the find words, regex, case, whole word and 「一致した行だけ」), so the search
    /// follows the person from one place to another. The next frame searches for it.
    pub fn take_query(&mut self, from: &LogView) {
        // `clear_query` moves the bar's version on, so a result for the old query is never taken for the new one.
        self.bar.clear_query();
        self.bar.find.clone_from(&from.bar.find);
        self.bar.regex = from.bar.regex;
        self.bar.match_case = from.bar.match_case;
        self.bar.whole_word = from.bar.whole_word;
        self.only_hits = from.only_hits;
        self.reset_search();
    }

    /// The search result for the text `gen` and the query as it is now, when there is one.
    fn valid(&self, gen: u64) -> Option<&Result<Found, String>> {
        valid_of(&self.done, self.bar.version(), self.bar.find.is_empty(), gen)
    }

    /// Takes the finished searches, sends the newest key when it is not the one in flight or done.
    fn run_search(&mut self, ctx: &egui::Context, text: &str, gen: u64) {
        if self.bar.find.is_empty() {
            if self.done.is_some() || self.sent.is_some() {
                self.reset_search();
            }
            return;
        }
        let key = (gen, self.bar.version());
        let mut dead = false;
        if let Some(w) = &self.worker {
            loop {
                match w.rx.try_recv() {
                    Ok(d) => {
                        if accept(key, (d.gen, d.version)) {
                            let total = d.found.as_ref().map_or(0, |f| f.hits.len());
                            if self.current_version == Some(d.version) {
                                self.current = keep(self.current, total);
                            } else {
                                self.current = 0;
                                self.current_version = Some(d.version);
                                self.scroll = true;
                            }
                            self.done = Some(d);
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        dead = true;
                        break;
                    }
                }
            }
        }
        if dead {
            // The thread is gone: start a new one with the key below instead of showing 検索しています… for ever.
            self.worker = None;
            self.sent = None;
        }
        let have = self.done.as_ref().map(|d| (d.gen, d.version));
        if have != Some(key) {
            if self.sent != Some(key) {
                let req = self.request(ctx, text, gen);
                self.worker.get_or_insert_with(Worker::start).shared.put(req);
                self.sent = Some(key);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// The request for the text `gen` and the query as it is now, from the viewport that is drawing.
    fn request(&self, ctx: &egui::Context, text: &str, gen: u64) -> Req {
        Req {
            gen,
            version: self.bar.version(),
            text: Arc::from(text),
            find: self.bar.find.clone(),
            query: self.bar.query(),
            ctx: ctx.clone(),
            viewport: ctx.viewport_id(),
        }
    }

    /// The bar's count follows the result.
    fn sync_status(&mut self, gen: u64) {
        let status = match self.valid(gen) {
            Some(Ok(f)) => Some(Ok(Status::Found { current: (!f.hits.is_empty()).then_some(self.current + 1), total: f.hits.len() })),
            Some(Err(e)) => Some(Err(e.clone())),
            None => None,
        };
        if let Some(s) = status {
            self.bar.set_result(self.bar.version(), s);
        }
    }

    /// Makes `self.drawn` what the body must show now: rebuilt when the text number, the query version or
    /// `only_hits` changed; kept while a search of the same query is pending for a newer text.
    fn update_drawn(&mut self, text: &str, gen: u64) {
        let version = self.bar.version();
        let query_on = !self.bar.find.is_empty();
        let res = valid_of(&self.done, version, !query_on, gen);
        // A result that does not fit the text (the caller reused a `gen`) is not drawn.
        let res = res.filter(|r| r.as_ref().ok().is_none_or(|f| fits(text, f)));
        let want_ver = matches!(res, Some(Ok(_))).then_some(version);
        let want_only = want_ver.is_some() && self.only_hits;
        let pending = res.is_none() && query_on;
        let keep_old = pending && self.drawn.as_ref().is_some_and(|d| d.version == Some(version) && d.only_hits == self.only_hits);
        let fresh = self.drawn.as_ref().is_some_and(|d| d.gen == gen && d.version == want_ver && d.only_hits == want_only);
        if keep_old || fresh {
            return;
        }
        #[cfg(test)]
        REBUILDS.with(|c| c.set((c.get().0 + 1, c.get().1)));
        let v = view(text, res, want_only);
        let (shown, hits) = (v.text.into_owned(), v.hits.into_owned());
        let mut starts = Vec::new();
        let mut at = 0;
        for l in shown.split_inclusive('\n') {
            starts.push(at);
            at += l.len();
        }
        self.serial += 1;
        self.drawn = Some(Drawn { serial: self.serial, gen, version: want_ver, only_hits: want_only, text: shown, hits, starts });
    }

    /// The finished save, once: the path written or the OS's words (a cancelled dialog says nothing). `show` calls it
    /// every frame; a caller whose window may not be drawn (a closed tab) calls it itself.
    pub fn poll_save(&mut self, ctx: &egui::Context) -> Option<Result<PathBuf, String>> {
        let rx = self.saving.as_ref()?;
        match rx.try_recv() {
            Ok(Some(r)) => {
                self.saving = None;
                Some(r)
            }
            Ok(None) => {
                self.saving = None;
                None
            }
            Err(TryRecvError::Disconnected) => {
                self.saving = None;
                Some(Err("the save thread ended".to_owned()))
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                None
            }
        }
    }

    /// Draws the viewer in the height the caller gives and says what happened.
    ///
    /// `gen` is the caller's number for `text`: it must change whenever `text` changes and must never come back to
    /// an old value with a different text (a result whose hits do not fit the text is dropped, but until then the
    /// highlight can be wrong).
    #[allow(clippy::too_many_arguments)]
    pub fn show(&mut self, ui: &mut egui::Ui, look: ito_prefs::Look, w: &Words, text: &str, gen: u64, save_name: &str) -> Out {
        let mut out = Out::default();
        self.bar.open = true;
        self.run_search(ui.ctx(), text, gen);
        self.sync_status(gen);

        // The head row.
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let mut copy = false;
        let mut save = false;
        let mut bo = ito_find::Output::default();
        ui.horizontal_wrapped(|ui| {
            bo = self.bar.show(ui, look, &w.find);
            let r = ui.checkbox(&mut self.only_hits, w.only_hits);
            ito_a11y::say(&r, Role::Button, w.only_hits);
            ito_a11y::test_id_in(ui, &r, &format!("{}.only_hits", self.ids), None);
            let r = ui.checkbox(&mut self.wrap, w.wrap);
            ito_a11y::say(&r, Role::Button, w.wrap);
            ito_a11y::test_id_in(ui, &r, &format!("{}.wrap", self.ids), None);
            let r = ito_prefs::IconButton::new(ito_prefs::Mark::Copy, w.copy).show(ui);
            ito_a11y::say(&r, Role::Button, w.copy);
            ito_a11y::test_id_in(ui, &r, &format!("{}.copy", self.ids), None);
            copy = r.clicked();
            let r = ui.button(w.save).on_hover_text(w.save_hint);
            ito_a11y::say(&r, Role::Button, w.save);
            ito_a11y::test_id_in(ui, &r, &format!("{}.save", self.ids), None);
            save = r.clicked();
        });

        // The bar: Esc with one of its widgets focused (or ✕) is `closed`: clear the query, keep the bar, and keep
        // that Esc from the caller. Esc elsewhere is the caller's.
        self.bar.open = true;
        if bo.closed {
            if esc {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            }
            self.bar.clear_query();
            self.reset_search();
            out.closed = true;
            ui.ctx().request_repaint();
        } else if bo.query_changed {
            ui.ctx().request_repaint();
        }
        let total = self.valid(gen).and_then(|r| r.as_ref().ok()).map_or(0, |f| f.hits.len());
        if total > 0 && (bo.next || bo.prev) {
            self.current = step(self.current, total, bo.next);
            self.scroll = true;
            ui.ctx().request_repaint();
        }

        // The body: one search result for the highlight, the lines shown and what is copied.
        self.update_drawn(text, gen);
        if copy {
            let shown = self.drawn.as_ref().map_or("", |d| d.text.as_str());
            out.copied = Some(ito_clipboard::clipboard!().set_text(shown).map_err(|e| e.to_string()));
        }
        if save && self.saving.is_none() {
            let owned = text.to_owned();
            let name = save_file_name(save_name);
            let title = w.save.trim_end_matches(['…', '.']).to_owned();
            self.saving = Some(ito_dialog::spawn(move || {
                let p = ito_dialog::Dialog::new().title(&title).file_name(&name).save_file()?;
                Some(std::fs::write(&p, owned).map(|()| p).map_err(|e| e.to_string()))
            }));
        }
        out.saved = self.poll_save(ui.ctx());

        let c = ito_theme::colors();
        if text.is_empty() {
            ui.label(egui::RichText::new(w.none).color(c.dim));
            return out;
        }
        let Some(d) = self.drawn.as_ref() else { return out };
        let want_scroll = std::mem::take(&mut self.scroll);
        let line = if want_scroll { scroll_line(&d.hits, self.current) } else { None };
        // A scroll asked for before there are hits waits for them.
        if want_scroll && line.is_none() && d.version.is_none() && !self.bar.find.is_empty() {
            self.scroll = true;
        }
        let follow = self.bar.find.is_empty();
        body(ui, &self.ids, &c, d, &mut self.galley, self.current, self.wrap, follow, line);
        out
    }
}

impl Drop for LogView {
    fn drop(&mut self) {
        if let Some(w) = &self.worker {
            w.shared.close();
        }
    }
}

// ---- drawing ----

fn bytes(r: Range<usize>) -> Range<egui::text::ByteIndex> {
    egui::text::ByteIndex(r.start)..egui::text::ByteIndex(r.end)
}

/// A text with the marked ranges (`true`: the current hit) as one layout job.
fn job_of(text: &str, marks: &[(Range<usize>, bool)], font: &egui::FontId, c: &ito_theme::Colors) -> LayoutJob {
    let plain = egui::TextFormat::simple(font.clone(), c.fg);
    let mut job = LayoutJob { text: text.to_owned(), ..Default::default() };
    let mut at = 0;
    for (r, cur) in marks {
        // Clamp to the text and to what is left after the last mark; a mark that becomes empty is dropped (a `\r`
        // that `lines()` took off can leave a hit one byte past its line).
        let (start, end) = (r.start.max(at), r.end.min(text.len()));
        if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        let r = &(start..end);
        if r.start > at {
            job.sections.push(LayoutSection { leading_space: 0.0, byte_range: bytes(at..r.start), format: plain.clone() });
        }
        let mut f = plain.clone();
        if *cur {
            f.background = c.accent();
            f.color = c.on_accent();
        } else {
            f.background = c.selection();
        }
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: bytes(r.clone()), format: f });
        at = r.end;
    }
    if at < text.len() {
        job.sections.push(LayoutSection { leading_space: 0.0, byte_range: bytes(at..text.len()), format: plain });
    }
    if job.sections.is_empty() {
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: bytes(0..0),
            format: egui::TextFormat::simple(font.clone(), c.fg),
        });
    }
    job
}

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut egui::Ui,
    ids: &str,
    c: &ito_theme::Colors,
    v: &Drawn,
    cache: &mut GalleyCache,
    current: usize,
    wrap: bool,
    follow: bool,
    scroll_to: Option<usize>,
) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    if wrap {
        // One job for the whole shown text, laid out again only when something it depends on changed.
        let ppp = ui.ctx().pixels_per_point().to_bits();
        egui::ScrollArea::vertical().id_salt(format!("{ids}.body.wrap")).auto_shrink(false).stick_to_bottom(follow).show(ui, |ui| {
            let key = GalleyKey {
                serial: v.serial,
                current,
                width: ui.available_width().to_bits(),
                ppp,
                font: font.clone(),
                colors: [c.fg, c.accent(), c.on_accent(), c.selection()],
            };
            let galley = match cache {
                Some((k, g)) if *k == key => g.clone(),
                _ => {
                    #[cfg(test)]
                    REBUILDS.with(|n| n.set((n.get().0, n.get().1 + 1)));
                    let marks: Vec<(Range<usize>, bool)> = v
                        .hits
                        .iter()
                        .enumerate()
                        .filter_map(|(i, h)| {
                            let s = *v.starts.get(h.line)?;
                            Some((s + h.range.start..s + h.range.end, i == current))
                        })
                        .collect();
                    let mut job = job_of(&v.text, &marks, &font, c);
                    job.wrap.max_width = ui.available_width();
                    let g = ui.painter().layout_job(job);
                    *cache = Some((key, g.clone()));
                    g
                }
            };
            let r = ui.add(egui::Label::new(egui::WidgetText::Galley(galley.clone())));
            if let (Some(h), true) = (v.hits.get(current), scroll_to.is_some()) {
                if let Some(prefix) = v.starts.get(h.line).and_then(|s| v.text.get(..s + h.range.start)) {
                    let rect = galley.pos_from_cursor(egui::text::CCursor::new(prefix.chars().count()));
                    ui.scroll_to_rect(rect.translate(r.rect.min.to_vec2()), Some(egui::Align::Center));
                }
            }
        });
    } else {
        let lines: Vec<&str> = v.text.lines().collect();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        let pitch = row_h + ui.spacing().item_spacing.y;
        let mut area = egui::ScrollArea::both().id_salt(format!("{ids}.body.long")).auto_shrink(false).stick_to_bottom(follow);
        if let Some(l) = scroll_to {
            area = area.vertical_scroll_offset((l as f32 * pitch - ui.available_height() / 2.0).max(0.0));
        }
        area.show_rows(ui, row_h, lines.len(), |ui, rows| {
            for i in rows {
                let s = v.hits.partition_point(|h| h.line < i);
                let e = v.hits.partition_point(|h| h.line <= i);
                let marks: Vec<(Range<usize>, bool)> = (s..e).map(|k| (v.hits[k].range.clone(), k == current)).collect();
                let job = job_of(lines[i], &marks, &font, c);
                ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> Query {
        Query { regex: false, whole_word: false, ignore_case: None, all: true }
    }

    fn hits(text: &str, find: &str, q: Query) -> Vec<(usize, Range<usize>)> {
        search(text, find, q).unwrap().hits.into_iter().map(|h| (h.line, h.range)).collect()
    }

    #[test]
    fn japanese_hits_are_on_char_boundaries() {
        let t = "開始 ジョブ\nジョブ終了 ジョブ";
        let h = hits(t, "ジョブ", q());
        assert_eq!(h.len(), 3);
        let lines: Vec<&str> = t.lines().collect();
        for (l, r) in &h {
            assert_eq!(&lines[*l][r.clone()], "ジョブ");
        }
        assert_eq!(h[0], (0, 7..16));
    }

    #[test]
    fn crlf_and_no_final_newline_and_empty_line() {
        let h = hits("ab\r\n\r\nab", "ab", q());
        assert_eq!(h, vec![(0, 0..2), (2, 0..2)]);
        // the \r is not part of the line, so a pattern on it finds nothing
        let r = Query { regex: true, ..q() };
        assert!(hits("ab\r\ncd", r"b\r", r).is_empty());
        // a last line with a bare \r keeps it (as `str::lines` does)
        assert_eq!(hits("x\nab\r", r"b\r", r), vec![(1, 1..3)]);
        assert!(hits("", "a", q()).is_empty());
    }

    #[test]
    fn empty_matches_are_not_hits() {
        let r = Query { regex: true, ..q() };
        assert!(hits("abc\n\nxyz", "^", r).is_empty());
        assert!(hits("abc", "z*", r).is_empty());
        assert_eq!(hits("baab", "a*", r), vec![(0, 1..3)]);
    }

    #[test]
    fn a_pattern_never_spans_lines() {
        let r = Query { regex: true, ..q() };
        assert!(hits("a\nb", r"a\nb", r).is_empty());
        assert!(hits("a\nb", "a\nb", q()).is_empty());
    }

    #[test]
    fn smart_case_match_case_and_whole_word() {
        let t = "cat Cat concat";
        assert_eq!(hits(t, "cat", q()).len(), 3);
        assert_eq!(hits(t, "Cat", q()).len(), 1);
        let exact = Query { ignore_case: Some(false), ..q() };
        assert_eq!(hits(t, "cat", exact).len(), 2);
        let word = Query { whole_word: true, ..q() };
        assert_eq!(hits(t, "cat", word), vec![(0, 0..3), (0, 4..7)]);
    }

    #[test]
    fn a_bad_regex_is_an_error_and_an_empty_query_has_no_hits() {
        let r = Query { regex: true, ..q() };
        assert!(search("a(", "(", r).is_err());
        assert_eq!(search("abc", "", q()), Ok(Found::default()));
    }

    #[test]
    fn next_and_prev_wrap_around() {
        assert_eq!(step(0, 3, true), 1);
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(1, 3, false), 0);
        assert_eq!(step(0, 0, true), 0);
        assert_eq!(step(0, 1, false), 0);
    }

    #[test]
    fn the_current_hit_is_kept_while_the_text_grows() {
        assert_eq!(keep(2, 5), 2);
        assert_eq!(keep(4, 5), 4);
        assert_eq!(keep(5, 5), 0);
        assert_eq!(keep(3, 0), 0);
    }

    #[test]
    fn only_matching_lines_and_what_copy_copies() {
        let t = "one\ntwo\r\nthree\nfour two";
        let ok = search(t, "two", q());
        let all = view(t, Some(&ok), false);
        assert_eq!(all.text, t);
        assert_eq!(all.hits.len(), 2);
        let only = view(t, Some(&ok), true);
        assert_eq!(only.text, "two\nfour two");
        assert_eq!(only.hits.iter().map(|h| (h.line, h.range.clone())).collect::<Vec<_>>(), vec![(0, 0..3), (1, 5..8)]);
        // a good query with no hit shows nothing when only_hits is on
        let none = search(t, "zzz", q());
        assert_eq!(view(t, Some(&none), true).text, "");
        assert_eq!(view(t, Some(&none), false).text, t);
        // a bad query, no result and an empty query: the whole text, only_hits ignored
        let bad = search(t, "(", Query { regex: true, ..q() });
        assert!(bad.is_err());
        for res in [Some(&bad), None] {
            let v = view(t, res, true);
            assert_eq!(v.text, t);
            assert!(v.hits.is_empty());
        }
    }

    #[test]
    fn the_line_to_scroll_to() {
        let t = "a\nb\nc a\nd\ne a";
        let ok = search(t, "a", q());
        let all = view(t, Some(&ok), false);
        assert_eq!([0, 1, 2].map(|i| scroll_line(&all.hits, i)), [Some(0), Some(2), Some(4)]);
        let only = view(t, Some(&ok), true);
        assert_eq!([0, 1, 2].map(|i| scroll_line(&only.hits, i)), [Some(0), Some(1), Some(2)]);
        assert_eq!(scroll_line(&all.hits, 3), None);
    }

    fn req(gen: u64, version: u64) -> Req {
        Req {
            gen,
            version,
            text: Arc::from("x"),
            find: "x".into(),
            query: q(),
            ctx: egui::Context::default(),
            viewport: egui::ViewportId::ROOT,
        }
    }

    #[test]
    fn a_result_for_an_older_key_is_dropped() {
        assert!(accept((3, 2), (3, 2)));
        assert!(!accept((3, 2), (2, 2)));
        assert!(!accept((3, 2), (3, 1)));
    }

    #[test]
    fn only_the_newest_request_is_kept() {
        let s = Shared::default();
        s.put(req(1, 1));
        s.put(req(2, 1));
        s.put(req(2, 2));
        let r = s.next().unwrap();
        assert_eq!((r.gen, r.version), (2, 2));
        assert!(s.slot.lock().unwrap().req.is_none());
        s.close();
        assert!(s.next().is_none());
    }

    #[test]
    fn the_worker_answers_and_stops() {
        let w = Worker::start();
        w.shared.put(Req {
            gen: 7,
            version: 3,
            text: Arc::from("a\nb a"),
            find: "a".into(),
            query: q(),
            ctx: egui::Context::default(),
            viewport: egui::ViewportId::ROOT,
        });
        let d = w.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!((d.gen, d.version), (7, 3));
        assert_eq!(d.found.unwrap().hits.len(), 2);
    }

    #[test]
    fn the_save_file_name() {
        assert_eq!(save_file_name("run 1"), "run 1.log");
        assert_eq!(save_file_name("a/b:c"), "a_b_c.log");
        assert_eq!(save_file_name("  "), "log.log");
        assert_eq!(save_file_name(""), "log.log");
        assert_eq!(save_file_name("日本語"), "日本語.log");
    }

    #[test]
    fn hits_that_do_not_fit_the_text_are_caught() {
        let ok = search("ab\ncd ab", "ab", q()).unwrap();
        assert!(fits("ab\ncd ab", &ok));
        assert!(!fits("ab", &ok));
        assert!(!fits("a", &ok));
        assert!(!fits("ab\nab", &ok));
    }

    #[test]
    fn marks_past_the_end_are_clamped() {
        let c = ito_theme::colors();
        let font = egui::FontId::monospace(12.0);
        // a mark one byte past the text, one wholly past it, an overlap and an empty one
        let job = job_of("ab", &[(1..3, false), (5..6, false), (0..2, true), (1..1, false)], &font, &c);
        let mut at = 0;
        for s in &job.sections {
            assert!(s.byte_range.start.0 >= at && s.byte_range.start.0 <= s.byte_range.end.0 && s.byte_range.end.0 <= 2);
            at = s.byte_range.end.0;
        }
        assert_eq!(at, 2);
        assert!(job_of("", &[(0..1, false)], &font, &c).sections.len() == 1);
    }

    const W: Words = Words {
        wrap: "wrap",
        only_hits: "only",
        copy: "copy",
        save: "save...",
        save_hint: "hint",
        none: "no log",
        find: ito_find::Words {
            find_hint: "Find",
            replace_hint: "Replace",
            regex: "Regex",
            match_case: "Case",
            whole_word: "Word",
            in_selection: "Sel",
            next: "Next",
            prev: "Prev",
            replace: "Replace",
            replace_all: "All",
            close: "Close",
            toggle_replace: "Toggle",
            no_results: "none",
            searching: "searching",
        },
    };

    fn frame(ctx: &egui::Context, v: &mut LogView, text: &str, gen: u64, events: Vec<egui::Event>) -> Out {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0))),
            events,
            ..Default::default()
        };
        let mut out = Out::default();
        let mut o = ctx.run_ui(input, |ui| {
            out = v.show(ui, ito_prefs::Look { c: ito_theme::colors(), q: "" }, &W, text, gen, "run 1");
        });
        o.textures_delta.clear();
        out
    }

    #[test]
    fn frames_run_for_every_case() {
        let ctx = egui::Context::default();
        // an empty text
        let mut v = LogView::new("run.log");
        frame(&ctx, &mut v, "", 0, vec![]);
        // a 2-line text with a query: wait for the worker's result
        let mut v = LogView::new("run.log");
        let text = "alpha beta\nbeta gamma";
        v.bar.find = "beta".into();
        v.bar.open_find(Some("beta"));
        let start = std::time::Instant::now();
        loop {
            frame(&ctx, &mut v, text, 1, vec![]);
            if matches!(v.bar.status, Status::Found { total: 2, current: Some(1) }) {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "no result: {:?}", v.bar.status);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // next goes to the second hit, wrapping later
        v.bar.status = Status::Found { current: Some(1), total: 2 };
        let o = v.bar.command(ito_find::cmd::NEXT, None);
        assert!(o.next);
        // wrap off, only the matching lines
        v.wrap = false;
        v.only_hits = true;
        frame(&ctx, &mut v, text, 1, vec![]);
        // a bad regex gives the bar's error
        let mut v = LogView::new("run.log");
        v.bar.regex = true;
        v.bar.open_find(Some("("));
        let start = std::time::Instant::now();
        loop {
            frame(&ctx, &mut v, text, 1, vec![]);
            if matches!(v.bar.status, Status::Error(_)) {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "no error: {:?}", v.bar.status);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn esc_in_the_box_clears_the_query_and_is_consumed() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.open_find(Some("beta"));
        let esc =
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&ctx, &mut v, "beta", 1, vec![]);
        let o = frame(&ctx, &mut v, "beta", 1, vec![esc]);
        assert!(o.closed);
        assert!(v.bar.open && v.bar.find.is_empty());
    }

    #[test]
    fn esc_without_the_box_focused_is_left_to_the_caller() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.find = "beta".into();
        let esc =
            egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&ctx, &mut v, "beta", 1, vec![]);
        let mut seen = false;
        let input = egui::RawInput { events: vec![esc], ..Default::default() };
        let mut o = ctx.run_ui(input, |ui| {
            let out = v.show(ui, ito_prefs::Look { c: ito_theme::colors(), q: "" }, &W, "beta", 1, "x");
            assert!(!out.closed);
            seen = ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
        o.textures_delta.clear();
        assert!(seen, "the caller still sees Esc");
        assert_eq!(v.bar.find, "beta");
    }

    fn esc_event() -> egui::Event {
        egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    /// Runs frames until `done` holds (the worker answers on its own thread).
    fn until(ctx: &egui::Context, v: &mut LogView, text: &str, gen: u64, done: impl Fn(&LogView) -> bool) {
        let start = std::time::Instant::now();
        while !done(v) {
            frame(ctx, v, text, gen, vec![]);
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "timed out: {:?}", v.bar.status);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn esc_in_another_text_box_is_not_taken() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.find = "beta".into();
        let other = egui::Id::new("other-box");
        let mut s = String::new();
        let mut run = |v: &mut LogView, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0))),
                events,
                ..Default::default()
            };
            let (mut closed, mut seen) = (false, false);
            let mut o = ctx.run_ui(input, |ui| {
                closed = v.show(ui, ito_prefs::Look { c: ito_theme::colors(), q: "" }, &W, "beta", 1, "x").closed;
                ui.add(egui::TextEdit::singleline(&mut s).id(other));
                seen = ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
            o.textures_delta.clear();
            (closed, seen)
        };
        run(&mut v, vec![]);
        ctx.memory_mut(|m| m.request_focus(other));
        run(&mut v, vec![]);
        run(&mut v, vec![]);
        assert!(ctx.text_edit_focused(), "the other box has focus");
        let (closed, seen) = run(&mut v, vec![esc_event()]);
        assert!(!closed && seen, "the other box keeps its Esc");
        assert_eq!(v.bar.find, "beta");
    }

    #[test]
    fn take_query_copies_the_search_and_moves_the_version_on() {
        let mut from = LogView::new("run.log");
        from.bar.find = "beta".into();
        from.bar.regex = true;
        from.bar.match_case = true;
        from.bar.whole_word = true;
        from.only_hits = true;
        let mut to = LogView::new("run.wide");
        let before = to.bar.version();
        to.take_query(&from);
        assert_eq!(
            (to.bar.find.as_str(), to.bar.regex, to.bar.match_case, to.bar.whole_word, to.only_hits),
            ("beta", true, true, true, true)
        );
        assert!(to.bar.version() > before);
        // Each keeps its own wrap.
        from.wrap = false;
        to.take_query(&from);
        assert!(to.wrap);
        // An empty query comes over as empty.
        let empty = LogView::new("run.log");
        to.take_query(&empty);
        assert!(to.bar.find.is_empty() && !to.only_hits && !to.bar.regex);
    }

    #[test]
    fn tab_to_a_toggle_then_esc_clears_and_is_not_the_callers() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.open_find(Some("beta"));
        let tab =
            egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&ctx, &mut v, "beta", 1, vec![]);
        frame(&ctx, &mut v, "beta", 1, vec![tab]);
        frame(&ctx, &mut v, "beta", 1, vec![]);
        assert!(!ctx.text_edit_focused(), "the focus moved to a button of the bar");
        let mut seen = true;
        let input = egui::RawInput { events: vec![esc_event()], ..Default::default() };
        let mut closed = false;
        let mut o = ctx.run_ui(input, |ui| {
            closed = v.show(ui, ito_prefs::Look { c: ito_theme::colors(), q: "" }, &W, "beta", 1, "x").closed;
            seen = ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
        o.textures_delta.clear();
        assert!(closed && !seen, "the bar took it");
        assert!(v.bar.open && v.bar.find.is_empty());
    }

    #[test]
    fn the_drawn_lines_stay_while_a_newer_text_is_searched() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.find = "beta".into();
        v.bar.open_find(Some("beta"));
        v.only_hits = true;
        let t1 = "alpha\nbeta\ngamma";
        until(&ctx, &mut v, t1, 1, |v| v.drawn.as_ref().is_some_and(|d| d.version.is_some()));
        assert_eq!(v.drawn.as_ref().unwrap().text, "beta");
        // a newer text: the worker has not answered yet, so the matching lines of gen 1 are still what is drawn (and copied)
        let t2 = "alpha\nbeta\ngamma\nbeta two";
        frame(&ctx, &mut v, t2, 2, vec![]);
        let d = v.drawn.as_ref().unwrap();
        assert_eq!((d.gen, d.text.as_str()), (1, "beta"));
        until(&ctx, &mut v, t2, 2, |v| v.drawn.as_ref().is_some_and(|d| d.gen == 2 && d.version.is_some()));
        assert_eq!(v.drawn.as_ref().unwrap().text, "beta\nbeta two");
    }

    #[test]
    fn an_idle_frame_rebuilds_nothing() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.find = "beta".into();
        v.bar.open_find(Some("beta"));
        let t = "alpha beta\nbeta gamma";
        until(&ctx, &mut v, t, 1, |v| v.drawn.as_ref().is_some_and(|d| d.version.is_some()));
        for _ in 0..3 {
            frame(&ctx, &mut v, t, 1, vec![]);
        }
        REBUILDS.with(|c| c.set((0, 0)));
        for _ in 0..3 {
            frame(&ctx, &mut v, t, 1, vec![]);
        }
        assert_eq!(REBUILDS.with(std::cell::Cell::get), (0, 0), "no view and no layout rebuilt");
        // a changed text does rebuild (once its search is done)
        until(&ctx, &mut v, "other beta", 2, |v| v.drawn.as_ref().is_some_and(|d| d.gen == 2 && d.version.is_some()));
        assert!(REBUILDS.with(std::cell::Cell::get).0 > 0);
    }

    #[test]
    fn a_hit_on_a_carriage_return_does_not_panic() {
        let ctx = egui::Context::default();
        for wrap in [true, false] {
            let mut v = LogView::new("run.log");
            v.wrap = wrap;
            v.only_hits = true;
            v.bar.regex = true;
            v.bar.find = r"b\r".into();
            v.bar.open_find(Some(r"b\r"));
            let t = "ab\r\r\nzz\nab\r\r\n";
            until(&ctx, &mut v, t, 1, |v| v.drawn.as_ref().is_some_and(|d| d.version.is_some()));
            assert_eq!(v.drawn.as_ref().unwrap().hits.len(), 2);
            frame(&ctx, &mut v, t, 1, vec![]);
        }
    }

    #[test]
    fn a_dead_worker_is_replaced() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        v.bar.find = "beta".into();
        v.bar.open_find(Some("beta"));
        // the worker's thread ends before it answers
        let w = Worker::start();
        w.shared.close();
        v.worker = Some(w);
        until(&ctx, &mut v, "beta", 1, |v| v.done.is_some());
        assert_eq!(v.done.as_ref().unwrap().found.as_ref().unwrap().hits.len(), 1);
    }

    #[test]
    fn a_search_request_names_the_viewport_it_came_from() {
        let ctx = egui::Context::default();
        let other = egui::ViewportId::from_hash_of("props.1");
        let mut v = LogView::new("props.log");
        v.bar.find = "beta".into();
        v.bar.open_find(Some("beta"));
        let mut got = Vec::new();
        // a frame of a viewport other than the root, then one of the root
        for id in [other, egui::ViewportId::ROOT] {
            let viewports = [(id, egui::ViewportInfo::default())].into_iter().collect();
            let mut out = ctx.run_ui(egui::RawInput { viewport_id: id, viewports, ..Default::default() }, |ui| {
                got.push(v.request(ui.ctx(), "beta", 1).viewport);
            });
            out.textures_delta.clear();
        }
        assert_eq!(got, [other, egui::ViewportId::ROOT]);
    }

    #[test]
    fn a_dead_save_thread_is_reported() {
        let ctx = egui::Context::default();
        let mut v = LogView::new("run.log");
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx);
        v.saving = Some(rx);
        let o = frame(&ctx, &mut v, "beta", 1, vec![]);
        assert!(matches!(o.saved, Some(Err(_))));
        assert!(v.saving.is_none());
        // a cancelled dialog stays silent
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(None).unwrap();
        v.saving = Some(rx);
        let o = frame(&ctx, &mut v, "beta", 1, vec![]);
        assert!(o.saved.is_none() && v.saving.is_none());
    }
}
