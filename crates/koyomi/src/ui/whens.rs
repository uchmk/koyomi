//! The `[[when]]` fields of the editor: a kind to choose, then the fields of
//! that kind as text, read back into a `When` on every change. A field that
//! does not read shows its reason in red and the net keeps the last good value.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use ito_i18n::Table;
use koyomi_core::duration;
use koyomi_core::when::{self, AfterOn, Kind, MonthRule, OnClosed, When};

/// The kinds in the order the chooser lists them (also the `when.kind_*` keys).
pub const KINDS: [&str; 14] =
    ["day", "week", "month", "year", "minutes", "hours", "once", "dates", "file", "log", "eventlog", "mail", "on_start", "after_jobnet"];

/// The rules of `every = "month"`.
pub const RULES: [&str; 4] = ["day", "last", "nth", "business_day"];

/// One `[[when]]` as the fields show it.
#[derive(Clone, Debug, PartialEq)]
pub struct WhenEdit {
    pub kind: String,
    /// Times, comma separated (`02:00, 14:30`).
    pub at: String,
    /// `once`: `2026-04-01 02:00`.
    pub once: String,
    pub n: String,
    pub from: String,
    pub to: String,
    /// Weekdays, comma separated (`mon, tue`).
    pub days: String,
    pub rule: String,
    pub day: String,
    pub shift: String,
    pub nth: String,
    pub weekday: String,
    /// `year`: `04-01`.
    pub date: String,
    /// Dates, comma separated (`2026-04-01, 2026-04-02`).
    pub dates: String,
    pub file: String,
    pub settle: String,
    /// `log`: the file and the text a line must hold.
    pub log: String,
    pub contains: String,
    /// `eventlog`: the log's name, ids comma separated, and the source.
    pub eventlog: String,
    pub ids: String,
    pub source: String,
    /// `mail`: the mailbox, the password as `secret:<name>`, and the texts
    /// the sender / subject hold.
    pub mail: String,
    pub password: String,
    pub sender: String,
    pub subject: String,
    pub jobnet: String,
    pub on: AfterOn,
    pub except: String,
    pub start: String,
    pub end: String,
    pub on_closed: Option<OnClosed>,
    /// `wait_for` and `wait_until`.
    pub wait_for: String,
    pub wait_until: String,
    /// Why the fields do not read, when they do not.
    pub error: Option<String>,
}

impl Default for WhenEdit {
    fn default() -> Self {
        WhenEdit {
            kind: "day".into(),
            at: "02:00".into(),
            once: String::new(),
            n: "30".into(),
            from: "09:00".into(),
            to: "18:00".into(),
            days: "mon, tue, wed, thu, fri".into(),
            rule: "day".into(),
            day: "1".into(),
            shift: "0".into(),
            nth: "1".into(),
            weekday: "mon".into(),
            date: "04-01".into(),
            dates: String::new(),
            file: String::new(),
            settle: String::new(),
            log: String::new(),
            contains: String::new(),
            eventlog: "Application".into(),
            ids: String::new(),
            source: String::new(),
            mail: String::new(),
            password: String::new(),
            sender: String::new(),
            subject: String::new(),
            jobnet: String::new(),
            on: AfterOn::Ok,
            except: String::new(),
            start: String::new(),
            end: String::new(),
            on_closed: None,
            wait_for: String::new(),
            wait_until: String::new(),
            error: None,
        }
    }
}

fn join<T: ToString>(items: &[T]) -> String {
    items.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
}

fn time_text(t: &NaiveTime) -> String {
    if t.format("%S").to_string() == "00" {
        t.format("%H:%M").to_string()
    } else {
        t.format("%H:%M:%S").to_string()
    }
}

fn weekday_text(w: &Weekday) -> String {
    format!("{w:?}").to_lowercase()
}

/// Splits a comma (or space) separated field, dropping the blanks.
fn items(s: &str) -> Vec<&str> {
    s.split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()).collect()
}

fn times(s: &str) -> Result<Vec<NaiveTime>, String> {
    let out: Result<Vec<_>, _> = items(s).into_iter().map(when::time).collect();
    let out = out?;
    if out.is_empty() {
        return Err("at は \"02:00\" のように時刻を 1 つ以上書く".into());
    }
    Ok(out)
}

fn weekdays(s: &str) -> Result<Vec<Weekday>, String> {
    items(s).into_iter().map(when::weekday).collect()
}

fn date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| format!("日付は \"2026-04-01\" のように書く: \"{}\"", s.trim()))
}

fn dates(s: &str) -> Result<Vec<NaiveDate>, String> {
    items(s).into_iter().map(date).collect()
}

fn number<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, String> {
    s.trim().parse().map_err(|_| format!("{what} は数で書く: \"{}\"", s.trim()))
}

impl WhenEdit {
    /// The fields for a kind, with its usual values.
    #[cfg(test)]
    pub fn new(kind: &str) -> Self {
        WhenEdit { kind: kind.into(), ..Default::default() }
    }

    /// The fields that show `w`.
    pub fn from_when(w: &When) -> Self {
        let mut e = WhenEdit::default();
        match &w.kind {
            Kind::Once { at } => {
                e.kind = "once".into();
                e.once = at.format("%Y-%m-%d %H:%M").to_string();
            }
            Kind::Day { at } => {
                e.kind = "day".into();
                e.at = join(&at.iter().map(time_text).collect::<Vec<_>>());
            }
            Kind::Minutes { n, from, to, days } | Kind::Hours { n, from, to, days } => {
                e.kind = if matches!(w.kind, Kind::Minutes { .. }) { "minutes" } else { "hours" }.into();
                e.n = n.to_string();
                e.from = time_text(from);
                e.to = time_text(to);
                e.days = join(&days.iter().map(weekday_text).collect::<Vec<_>>());
            }
            Kind::Week { days, at } => {
                e.kind = "week".into();
                e.days = join(&days.iter().map(weekday_text).collect::<Vec<_>>());
                e.at = join(&at.iter().map(time_text).collect::<Vec<_>>());
            }
            Kind::Month { rule, at } => {
                e.kind = "month".into();
                match rule {
                    MonthRule::Day { day, shift } => {
                        e.rule = "day".into();
                        e.day = day.to_string();
                        e.shift = shift.to_string();
                    }
                    MonthRule::Last => e.rule = "last".into(),
                    MonthRule::Nth { nth, weekday } => {
                        e.rule = "nth".into();
                        e.nth = nth.to_string();
                        e.weekday = weekday_text(weekday);
                    }
                    MonthRule::BusinessDay { n } => {
                        e.rule = "business_day".into();
                        e.n = n.to_string();
                    }
                }
                e.at = join(&at.iter().map(time_text).collect::<Vec<_>>());
            }
            Kind::Year { month, day, at } => {
                e.kind = "year".into();
                e.date = format!("{month:02}-{day:02}");
                e.at = join(&at.iter().map(time_text).collect::<Vec<_>>());
            }
            Kind::Dates { dates, at } => {
                e.kind = "dates".into();
                e.dates = join(&dates.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect::<Vec<_>>());
                e.at = join(&at.iter().map(time_text).collect::<Vec<_>>());
            }
            Kind::File { file, settle } => {
                e.kind = "file".into();
                e.file = file.clone();
                e.settle = settle.map(duration::text).unwrap_or_default();
            }
            Kind::Log { log, contains } => {
                e.kind = "log".into();
                e.log = log.clone();
                e.contains = contains.clone();
            }
            Kind::EventLog { eventlog, ids, source } => {
                e.kind = "eventlog".into();
                e.eventlog = eventlog.clone();
                e.ids = join(ids);
                e.source = source.clone().unwrap_or_default();
            }
            Kind::Mail { mail, secret, sender, subject } => {
                e.kind = "mail".into();
                e.mail = mail.clone();
                e.password = format!("secret:{secret}");
                e.sender = sender.clone().unwrap_or_default();
                e.subject = subject.clone().unwrap_or_default();
            }
            Kind::OnStart => e.kind = "on_start".into(),
            Kind::AfterJobnet { jobnet, on } => {
                e.kind = "after_jobnet".into();
                e.jobnet = jobnet.clone();
                e.on = *on;
            }
        }
        e.except = join(&w.except.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect::<Vec<_>>());
        e.start = w.start.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
        e.end = w.end.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
        e.on_closed = w.on_closed;
        e.wait_for = w.wait_for.clone().unwrap_or_default();
        e.wait_until = w.wait_until.map(|t| time_text(&t)).unwrap_or_default();
        e
    }

    /// The `When` the fields say, or why they do not.
    pub fn to_when(&self) -> Result<When, String> {
        let kind = match self.kind.as_str() {
            "once" => Kind::Once {
                at: NaiveDateTime::parse_from_str(self.once.trim(), "%Y-%m-%d %H:%M")
                    .or_else(|_| NaiveDateTime::parse_from_str(self.once.trim(), "%Y-%m-%d %H:%M:%S"))
                    .map_err(|_| format!("once は \"2026-04-01 02:00\" のように書く: \"{}\"", self.once.trim()))?,
            },
            "day" => Kind::Day { at: times(&self.at)? },
            "minutes" | "hours" => {
                let n: u32 = number(&self.n, "n")?;
                if n == 0 {
                    return Err("n は 1 以上".into());
                }
                let (from, to, days) = (when::time(&self.from)?, when::time(&self.to)?, weekdays(&self.days)?);
                if self.kind == "minutes" {
                    Kind::Minutes { n, from, to, days }
                } else {
                    Kind::Hours { n, from, to, days }
                }
            }
            "week" => {
                let days = weekdays(&self.days)?;
                if days.is_empty() {
                    return Err("days は曜日を 1 つ以上書く".into());
                }
                Kind::Week { days, at: times(&self.at)? }
            }
            "month" => {
                let rule = match self.rule.as_str() {
                    "day" => {
                        let day: u32 = number(&self.day, "day")?;
                        if !(1..=31).contains(&day) {
                            return Err("day は 1〜31".into());
                        }
                        MonthRule::Day { day, shift: if self.shift.trim().is_empty() { 0 } else { number(&self.shift, "shift")? } }
                    }
                    "last" => MonthRule::Last,
                    "nth" => {
                        let nth: u32 = number(&self.nth, "nth")?;
                        if !(1..=5).contains(&nth) {
                            return Err("nth は 1〜5".into());
                        }
                        MonthRule::Nth { nth, weekday: when::weekday(&self.weekday)? }
                    }
                    _ => {
                        let n: i32 = number(&self.n, "business_day")?;
                        if n == 0 {
                            return Err("business_day は 0 以外（-1 は最終営業日）".into());
                        }
                        MonthRule::BusinessDay { n }
                    }
                };
                Kind::Month { rule, at: times(&self.at)? }
            }
            "year" => {
                let (month, day) = when::month_day(&self.date)?;
                Kind::Year { month, day, at: times(&self.at)? }
            }
            "dates" => {
                let ds = dates(&self.dates)?;
                if ds.is_empty() {
                    return Err("dates は日付を 1 つ以上書く".into());
                }
                Kind::Dates { dates: ds, at: times(&self.at)? }
            }
            "file" => {
                if self.file.trim().is_empty() {
                    return Err("file は待つファイルのパス".into());
                }
                let settle = if self.settle.trim().is_empty() { None } else { Some(duration::parse(&self.settle)?) };
                Kind::File { file: self.file.trim().into(), settle }
            }
            "log" => {
                if self.log.trim().is_empty() {
                    return Err("log は見るファイルのパス".into());
                }
                if self.contains.is_empty() {
                    return Err("contains は行に含まれる文字（例: ERROR）".into());
                }
                Kind::Log { log: self.log.trim().into(), contains: self.contains.clone() }
            }
            "eventlog" => {
                if self.eventlog.trim().is_empty() {
                    return Err("eventlog はログの名前（例: Application）".into());
                }
                let ids = items(&self.ids)
                    .into_iter()
                    .map(|s| s.parse::<u32>().ok().filter(|n| *n <= 65535).ok_or(format!("ids はイベント ID（0〜65535）: \"{s}\"")))
                    .collect::<Result<Vec<_>, _>>()?;
                let source = Some(self.source.trim()).filter(|s| !s.is_empty()).map(str::to_owned);
                if ids.is_empty() && source.is_none() {
                    return Err("eventlog には ids か source が要る".into());
                }
                Kind::EventLog { eventlog: self.eventlog.trim().into(), ids, source }
            }
            "mail" => {
                when::mailbox(&self.mail)?;
                let secret = match self.password.trim().strip_prefix("secret:") {
                    Some(n) if !n.trim().is_empty() => n.to_owned(),
                    _ => return Err("password は \"secret:<名前>\" で書く（koyomi secret set <名前> で OS の資格情報に置く）".into()),
                };
                let text = |s: &str| Some(s.to_owned()).filter(|s| !s.trim().is_empty());
                let (sender, subject) = (text(&self.sender), text(&self.subject));
                if sender.is_none() && subject.is_none() {
                    return Err("mail には sender か subject が要る".into());
                }
                Kind::Mail { mail: self.mail.trim().into(), secret, sender, subject }
            }
            "on_start" => Kind::OnStart,
            "after_jobnet" => {
                if !koyomi_core::jobnet::is_id(self.jobnet.trim()) {
                    return Err(format!("after_jobnet は \"グループ/名前\": \"{}\"", self.jobnet.trim()));
                }
                Kind::AfterJobnet { jobnet: self.jobnet.trim().into(), on: self.on }
            }
            k => return Err(format!("知らない種類: {k}")),
        };
        let opt = |s: &str| -> Result<Option<NaiveDate>, String> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                date(s).map(Some)
            }
        };
        let wait_for = match self.wait_for.trim() {
            "" => None,
            u if koyomi_core::jobnet::is_id(u) => Some(u.to_owned()),
            u => return Err(format!("wait_for は \"グループ/名前\": \"{u}\"")),
        };
        let wait_until = if self.wait_until.trim().is_empty() { None } else { Some(when::time(&self.wait_until)?) };
        if wait_until.is_some() && wait_for.is_none() {
            return Err("wait_until は wait_for と一緒に書く".into());
        }
        Ok(When {
            kind,
            except: dates(&self.except)?,
            start: opt(&self.start)?,
            end: opt(&self.end)?,
            on_closed: self.on_closed,
            wait_for,
            wait_until,
        })
    }

    /// The fields; `true` when one changed.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &Table, i: usize) -> bool {
        let mut changed = false;
        let field = |ui: &mut egui::Ui, label: &str, s: &mut String, width: f32| -> bool {
            let mut hit = false;
            ui.horizontal(|ui| {
                ui.label(label);
                let r = ui.add(egui::TextEdit::singleline(s).desired_width(width));
                ito_a11y::say(&r, ito_a11y::Role::Label, label);
                hit = r.changed();
            });
            hit
        };
        ui.horizontal(|ui| {
            ui.label(t.tr("when.kind"));
            egui::ComboBox::from_id_salt(("when-kind", i)).selected_text(t.tr(&format!("when.kind_{}", self.kind))).show_ui(ui, |ui| {
                for k in KINDS {
                    if ui.selectable_label(self.kind == k, t.tr(&format!("when.kind_{k}"))).clicked() && self.kind != k {
                        self.kind = k.into();
                        changed = true;
                    }
                }
            });
        });
        match self.kind.as_str() {
            "once" => changed |= field(ui, t.tr("when.once"), &mut self.once, 160.0),
            "day" => changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0),
            "minutes" | "hours" => {
                changed |= field(ui, t.tr("when.n"), &mut self.n, 50.0);
                changed |= field(ui, t.tr("when.from"), &mut self.from, 70.0);
                changed |= field(ui, t.tr("when.to"), &mut self.to, 70.0);
                changed |= field(ui, t.tr("when.days"), &mut self.days, 160.0);
            }
            "week" => {
                changed |= field(ui, t.tr("when.days"), &mut self.days, 160.0);
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "month" => {
                ui.horizontal(|ui| {
                    ui.label(t.tr("when.rule"));
                    egui::ComboBox::from_id_salt(("when-rule", i)).selected_text(t.tr(&format!("when.rule_{}", self.rule))).show_ui(
                        ui,
                        |ui| {
                            for r in RULES {
                                if ui.selectable_label(self.rule == r, t.tr(&format!("when.rule_{r}"))).clicked() && self.rule != r {
                                    self.rule = r.into();
                                    changed = true;
                                }
                            }
                        },
                    );
                });
                match self.rule.as_str() {
                    "day" => {
                        changed |= field(ui, t.tr("when.day"), &mut self.day, 50.0);
                        changed |= field(ui, t.tr("when.shift"), &mut self.shift, 50.0);
                    }
                    "nth" => {
                        changed |= field(ui, t.tr("when.nth"), &mut self.nth, 50.0);
                        changed |= field(ui, t.tr("when.weekday"), &mut self.weekday, 70.0);
                    }
                    "business_day" => changed |= field(ui, t.tr("when.business_day"), &mut self.n, 50.0),
                    _ => {}
                }
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "year" => {
                changed |= field(ui, t.tr("when.date"), &mut self.date, 70.0);
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "dates" => {
                changed |= field(ui, t.tr("when.dates"), &mut self.dates, 200.0);
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "file" => {
                changed |= field(ui, t.tr("when.file"), &mut self.file, 200.0);
                changed |= field(ui, t.tr("when.settle"), &mut self.settle, 70.0);
            }
            "log" => {
                changed |= field(ui, t.tr("when.log"), &mut self.log, 200.0);
                changed |= field(ui, t.tr("when.contains"), &mut self.contains, 160.0);
            }
            "eventlog" => {
                changed |= field(ui, t.tr("when.eventlog"), &mut self.eventlog, 120.0);
                changed |= field(ui, t.tr("when.ids"), &mut self.ids, 120.0);
                changed |= field(ui, t.tr("when.source"), &mut self.source, 160.0);
            }
            "mail" => {
                changed |= field(ui, t.tr("when.mail"), &mut self.mail, 240.0);
                changed |= field(ui, t.tr("when.password"), &mut self.password, 120.0);
                changed |= field(ui, t.tr("when.sender"), &mut self.sender, 160.0);
                changed |= field(ui, t.tr("when.subject"), &mut self.subject, 160.0);
            }
            "after_jobnet" => {
                changed |= field(ui, t.tr("when.jobnet"), &mut self.jobnet, 160.0);
                ui.horizontal(|ui| {
                    ui.label(t.tr("when.on"));
                    for (on, key) in
                        [(AfterOn::Ok, "canvas.line_ok"), (AfterOn::Error, "canvas.line_error"), (AfterOn::Any, "canvas.line_any")]
                    {
                        if ui.selectable_label(self.on == on, t.tr(key)).clicked() && self.on != on {
                            self.on = on;
                            changed = true;
                        }
                    }
                });
            }
            _ => {}
        }
        if !matches!(self.kind.as_str(), "on_start" | "after_jobnet" | "file" | "log" | "eventlog" | "mail") {
            changed |= field(ui, t.tr("when.except"), &mut self.except, 200.0);
            changed |= field(ui, t.tr("when.start"), &mut self.start, 90.0);
            changed |= field(ui, t.tr("when.end"), &mut self.end, 90.0);
            changed |= field(ui, t.tr("when.wait_for"), &mut self.wait_for, 200.0);
            changed |= field(ui, t.tr("when.wait_until"), &mut self.wait_until, 70.0);
            ui.horizontal(|ui| {
                ui.label(t.tr("when.on_closed"));
                let choices = [
                    (None, "when.closed_default"),
                    (Some(OnClosed::Prev), "when.closed_prev"),
                    (Some(OnClosed::Next), "when.closed_next"),
                    (Some(OnClosed::Skip), "when.closed_skip"),
                    (Some(OnClosed::Run), "when.closed_run"),
                ];
                for (c, key) in choices {
                    if ui.selectable_label(self.on_closed == c, t.tr(key)).clicked() && self.on_closed != c {
                        self.on_closed = c;
                        changed = true;
                    }
                }
            });
        }
        if let Some(e) = &self.error {
            ui.colored_label(ito_theme::colors().err, e);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_round_trip_every_kind() {
        let text = "format = 1\n[[when]]\nevery = \"day\"\nat = [\"02:00\", \"14:30:15\"]\nexcept = [\"2026-05-01\"]\nstart = \"2026-04-01\"\non_closed = \"prev\"\n\
            [[when]]\nevery = \"minutes\"\nn = 15\nfrom = \"09:00\"\nto = \"18:00\"\ndays = [\"mon\", \"fri\"]\n\
            [[when]]\nevery = \"week\"\ndays = [\"tue\"]\nat = [\"03:00\"]\n\
            [[when]]\nevery = \"month\"\nday = 25\nshift = -1\nat = [\"04:00\"]\n\
            [[when]]\nevery = \"month\"\nnth = 2\nweekday = \"wed\"\nat = [\"04:00\"]\n\
            [[when]]\nevery = \"month\"\nbusiness_day = -1\nat = [\"04:00\"]\n\
            [[when]]\nevery = \"month\"\nday = \"last\"\nat = [\"04:00\"]\n\
            [[when]]\nevery = \"year\"\ndate = \"04-01\"\nat = [\"05:00\"]\n\
            [[when]]\ndates = [\"2026-04-01\"]\nat = [\"06:00\"]\n\
            [[when]]\nonce = 2026-04-01T07:00:00\n\
            [[when]]\nfile = \"C:/in/x.csv\"\nsettle = \"30s\"\n\
            [[when]]\nlog = 'D:\\app\\a.log'\ncontains = \"ERROR\"\n\
            [[when]]\neventlog = \"Security\"\nids = [4625, 4740]\n\
            [[when]]\neventlog = \"Application\"\nsource = \"MyApp\"\n\
            [[when]]\nmail = \"imaps://me@example.com@imap.example.com/INBOX\"\npassword = \"secret:mail\"\nsender = \"billing@\"\nsubject = \"請求\"\n\
            [[when]]\non_start = true\n\
            [[when]]\nafter_jobnet = \"経理/集計\"\non = \"error\"\n\
            [[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        let net = koyomi_core::jobnet::parse(text).unwrap();
        assert_eq!(net.when.len(), 17);
        for w in &net.when {
            let e = WhenEdit::from_when(w);
            assert_eq!(e.to_when().as_ref(), Ok(w), "{e:?}");
        }
        assert_eq!(WhenEdit::from_when(&net.when[0]).at, "02:00, 14:30:15");
    }

    #[test]
    fn a_field_that_does_not_read_says_why() {
        let mut e = WhenEdit::new("day");
        e.at = "25:00".into();
        assert!(e.to_when().is_err());
        e.at = String::new();
        assert!(e.to_when().unwrap_err().contains("at"));
        let mut e = WhenEdit::new("month");
        e.rule = "day".into();
        e.day = "32".into();
        assert!(e.to_when().unwrap_err().contains("day"));
        let mut e = WhenEdit::new("after_jobnet");
        e.jobnet = "集計".into();
        assert!(e.to_when().is_err());
        e.jobnet = "経理/集計".into();
        assert!(e.to_when().is_ok());
        let mut e = WhenEdit::new("day");
        e.start = "2026/04/01".into();
        assert!(e.to_when().unwrap_err().contains("2026-04-01"));
        // The usual values of every kind read as they are.
        for k in KINDS {
            let mut e = WhenEdit::new(k);
            e.file = "x".into();
            e.log = "x".into();
            e.contains = "x".into();
            e.ids = "1".into();
            e.jobnet = "a/b".into();
            e.mail = "imaps://a@b".into();
            e.password = "secret:m".into();
            e.subject = "x".into();
            e.once = "2026-04-01 02:00".into();
            e.dates = "2026-04-01".into();
            assert!(e.to_when().is_ok(), "{k}: {:?}", e.to_when());
        }
    }
}
