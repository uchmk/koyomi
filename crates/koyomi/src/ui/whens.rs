//! The `[[when]]` fields of the editor: a kind to choose, then the fields of
//! that kind as text, read back into a `When` on every change. A field that
//! does not read shows its reason in red and the net keeps the last good value.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use ito_i18n::Table;
use koyomi_core::duration;
use koyomi_core::msg;
use koyomi_core::when::{self, AfterOn, Kind, MonthRule, OnClosed, Session, When};

use super::pick::{self, Ctx, Want};

/// The kinds in the order the chooser lists them (also the `when.kind_*` keys).
pub const KINDS: [&str; 16] = [
    "day",
    "week",
    "month",
    "year",
    "minutes",
    "hours",
    "once",
    "dates",
    "file",
    "log",
    "eventlog",
    "mail",
    "on_start",
    "idle",
    "session",
    "after_jobnet",
];

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
    /// `mailbox`: a `[[mailbox]]` of koyomi.toml, which owns `mail` and `password`.
    pub mailbox: String,
    pub password: String,
    pub sender: String,
    pub subject: String,
    /// `idle`: how long the PC is left alone (`10m`).
    pub idle: String,
    /// `session`: the sign-in, sign-out, lock or unlock that starts the net.
    pub session: Session,
    /// `user`: only that person's session (`PC\taro`); empty: anyone's.
    pub user: String,
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
            mailbox: String::new(),
            password: String::new(),
            sender: String::new(),
            subject: String::new(),
            idle: "10m".into(),
            session: Session::Lock,
            user: String::new(),
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
        return Err(msg::line(msg::DEF_WHEN_KEY_MISSING, &["at", "at = [\"02:00\"]"]));
    }
    Ok(out)
}

fn weekdays(s: &str) -> Result<Vec<Weekday>, String> {
    items(s).into_iter().map(when::weekday).collect()
}

fn date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| msg::line(msg::DATE_BAD, &[s.trim()]))
}

fn dates(s: &str) -> Result<Vec<NaiveDate>, String> {
    items(s).into_iter().map(date).collect()
}

/// The number in `s`, when it is one.
fn number<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().parse().ok()
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
            Kind::Mail { mail, secret, mailbox, sender, subject } => {
                e.kind = "mail".into();
                e.mail = mail.clone();
                e.mailbox = mailbox.clone().unwrap_or_default();
                if mailbox.is_none() {
                    e.password = format!("secret:{secret}");
                }
                e.sender = sender.clone().unwrap_or_default();
                e.subject = subject.clone().unwrap_or_default();
            }
            Kind::OnStart => e.kind = "on_start".into(),
            Kind::Idle { idle } => {
                e.kind = "idle".into();
                e.idle = duration::text(*idle);
            }
            Kind::Session { session, user } => {
                e.kind = "session".into();
                e.session = *session;
                e.user = user.clone().unwrap_or_default();
            }
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
                    .map_err(|_| msg::line(msg::WIN_DATETIME_BAD, &[self.once.trim()]))?,
            },
            "day" => Kind::Day { at: times(&self.at)? },
            "minutes" | "hours" => {
                let n: u32 = number(&self.n).filter(|n| *n >= 1).ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["n", "1", "1440"]))?;
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
                    return Err(msg::line(msg::DEF_WHEN_KEY_MISSING, &["days", "days = [\"mon\", \"fri\"]"]));
                }
                Kind::Week { days, at: times(&self.at)? }
            }
            "month" => {
                let rule = match self.rule.as_str() {
                    "day" => {
                        let day: u32 = number(&self.day)
                            .filter(|d| (1..=31).contains(d))
                            .ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["day", "1", "31"]))?;
                        let shift = if self.shift.trim().is_empty() {
                            0
                        } else {
                            number(&self.shift).ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["shift", "-31", "31"]))?
                        };
                        MonthRule::Day { day, shift }
                    }
                    "last" => MonthRule::Last,
                    "nth" => {
                        let nth: u32 = number(&self.nth)
                            .filter(|n| (1..=5).contains(n))
                            .ok_or_else(|| msg::line(msg::DEF_INT_RANGE, &["nth", "1", "5"]))?;
                        MonthRule::Nth { nth, weekday: when::weekday(&self.weekday)? }
                    }
                    _ => {
                        let n: i32 = number(&self.n).filter(|n| *n != 0).ok_or_else(|| msg::line(msg::DEF_BUSINESS_DAY_BAD, &[]))?;
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
                    return Err(msg::line(msg::DEF_VALUE_EMPTY, &["dates"]));
                }
                Kind::Dates { dates: ds, at: times(&self.at)? }
            }
            "file" => {
                if self.file.trim().is_empty() {
                    return Err(msg::line(msg::DEF_VALUE_EMPTY, &["file"]));
                }
                let settle = if self.settle.trim().is_empty() { None } else { Some(duration::parse(&self.settle)?) };
                Kind::File { file: self.file.trim().into(), settle }
            }
            "log" => {
                if self.log.trim().is_empty() {
                    return Err(msg::line(msg::DEF_VALUE_EMPTY, &["log"]));
                }
                if self.contains.is_empty() {
                    return Err(msg::line(msg::DEF_WHEN_KEY_MISSING, &["contains", "contains = \"ERROR\""]));
                }
                Kind::Log { log: self.log.trim().into(), contains: self.contains.clone() }
            }
            "eventlog" => {
                if self.eventlog.trim().is_empty() {
                    return Err(msg::line(msg::DEF_WHEN_KEY_MISSING, &["eventlog", "eventlog = \"Application\""]));
                }
                let ids = items(&self.ids)
                    .into_iter()
                    .map(|s| s.parse::<u32>().ok().filter(|n| *n <= 65535).ok_or_else(|| msg::line(msg::DEF_IDS_BAD, &[])))
                    .collect::<Result<Vec<_>, _>>()?;
                let source = Some(self.source.trim()).filter(|s| !s.is_empty()).map(str::to_owned);
                if ids.is_empty() && source.is_none() {
                    return Err(msg::line(msg::DEF_EITHER_REQUIRED, &["eventlog", "ids", "source", "ids = [4625]"]));
                }
                Kind::EventLog { eventlog: self.eventlog.trim().into(), ids, source }
            }
            "mail" if !self.mailbox.trim().is_empty() => {
                let text = |s: &str| Some(s.to_owned()).filter(|s| !s.trim().is_empty());
                let (sender, subject) = (text(&self.sender), text(&self.subject));
                if sender.is_none() && subject.is_none() {
                    return Err(msg::line(msg::DEF_EITHER_REQUIRED, &["mail", "sender", "subject", "subject = \"Invoice\""]));
                }
                Kind::Mail { mail: String::new(), secret: String::new(), mailbox: Some(self.mailbox.trim().into()), sender, subject }
            }
            "mail" => {
                when::mailbox(&self.mail)?;
                let secret = match self.password.trim().strip_prefix("secret:") {
                    Some(n) if !n.trim().is_empty() => n.to_owned(),
                    Some(_) => return Err(msg::line(msg::DEF_SECRET_NAME_EMPTY, &["password"])),
                    None => return Err(msg::line(msg::DEF_PASSWORD_NOT_SECRET, &["password"])),
                };
                let text = |s: &str| Some(s.to_owned()).filter(|s| !s.trim().is_empty());
                let (sender, subject) = (text(&self.sender), text(&self.subject));
                if sender.is_none() && subject.is_none() {
                    return Err(msg::line(msg::DEF_EITHER_REQUIRED, &["mail", "sender", "subject", "subject = \"Invoice\""]));
                }
                Kind::Mail { mail: self.mail.trim().into(), secret, mailbox: None, sender, subject }
            }
            "on_start" => Kind::OnStart,
            "idle" => {
                let idle = duration::parse(&self.idle)?;
                if idle < 60 {
                    return Err(msg::line(msg::DEF_IDLE_SHORT, &[]));
                }
                Kind::Idle { idle }
            }
            "session" => {
                let user = Some(self.user.trim().to_owned()).filter(|u| !u.is_empty());
                if user.as_deref().is_some_and(|u| !koyomi_core::when::valid_session_user(u)) {
                    return Err(msg::line(msg::DEF_SESSION_USER_BAD, &[]));
                }
                Kind::Session { session: self.session, user }
            }
            "after_jobnet" => {
                if !koyomi_core::jobnet::is_id(self.jobnet.trim()) {
                    return Err(msg::line(msg::DEF_JOBNET_REF_BAD, &["after_jobnet"]));
                }
                Kind::AfterJobnet { jobnet: self.jobnet.trim().into(), on: self.on }
            }
            k => return Err(msg::line(msg::WIN_WHEN_KIND_UNKNOWN, &[k])),
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
            _ => return Err(msg::line(msg::DEF_JOBNET_REF_BAD, &["wait_for"])),
        };
        let wait_until = if self.wait_until.trim().is_empty() { None } else { Some(when::time(&self.wait_until)?) };
        if wait_until.is_some() && wait_for.is_none() {
            return Err(msg::line(msg::DEF_WAIT_UNTIL_ALONE, &[]));
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

    /// The fields; `true` when one changed. `pick` holds the nets and the file chooser.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &Table, i: usize, pick: &Ctx) -> bool {
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
        // A field that has a picker beside it: typed only while its text does not read (or while it is typed in), under
        // an id of its own so the picker can wait for the person to leave it.
        let typed_id = |key: &str| egui::Id::new(("when-typed", key.to_owned()));
        let typing = |ui: &egui::Ui, key: &str| ui.memory(|m| m.has_focus(typed_id(key)));
        let typed = |ui: &mut egui::Ui, label: &str, s: &mut String, width: f32, key: &str| -> bool {
            let mut hit = false;
            ui.horizontal(|ui| {
                ui.label(label);
                let r = ui.add(egui::TextEdit::singleline(s).desired_width(width).id(typed_id(key)));
                ito_a11y::say(&r, ito_a11y::Role::Label, label);
                hit = r.changed();
            });
            hit
        };
        // A path field with 参照… at its right (on this PC's engine).
        let path = |ui: &mut egui::Ui, label: &str, s: &mut String, width: f32, key: &str, want: Want| -> bool {
            let mut hit = false;
            ui.horizontal(|ui| {
                ui.label(label);
                let r = ui.add(egui::TextEdit::singleline(s).desired_width(width));
                ito_a11y::say(&r, ito_a11y::Role::Label, label);
                hit = r.changed();
                hit |= pick.browse_button(ui, t, &format!("{key}-{i}"), label, want, s);
            });
            hit
        };
        // A label on its own line, then the choices.
        let named = |ui: &mut egui::Ui, label: &str| {
            let r = ui.label(label);
            ito_a11y::say(&r, ito_a11y::Role::Label, label);
        };
        // A number out of a short list, when the text is one of them and is not being typed.
        let nth = |ui: &mut egui::Ui, label: &str, s: &mut String| -> bool {
            let key = format!("nth-{i}");
            match pick::number_in(s, 1, 5).filter(|_| !typing(ui, &key)) {
                Some(n) => {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let words: Vec<String> = (1..=5).map(|n| t.tr(&format!("pick.nth_{n}")).to_owned()).collect();
                        match pick::select_index(ui, &format!("when-nth-{i}"), label, n as usize - 1, &words) {
                            Some(k) => {
                                *s = (k + 1).to_string();
                                true
                            }
                            None => false,
                        }
                    })
                    .inner
                }
                None => typed(ui, label, s, 50.0, &key),
            }
        };
        let weekday = |ui: &mut egui::Ui, label: &str, s: &mut String| -> bool {
            let key = format!("weekday-{i}");
            match pick::weekday_index(s).filter(|_| !typing(ui, &key)) {
                Some(at) => {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let words: Vec<String> = pick::DAYS.iter().map(|d| t.tr(&format!("weekday.{d}")).to_owned()).collect();
                        match pick::select_index(ui, &format!("when-weekday-{i}"), label, at, &words) {
                            Some(k) => {
                                *s = pick::DAYS[k].to_owned();
                                true
                            }
                            None => false,
                        }
                    })
                    .inner
                }
                None => typed(ui, label, s, 70.0, &key),
            }
        };
        let month_day = |ui: &mut egui::Ui, label: &str, s: &mut String| -> bool {
            let key = format!("day-{i}");
            match pick::number_in(s, 1, 31).filter(|_| !typing(ui, &key)) {
                Some(n) => {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let words: Vec<String> = (1..=31).map(|n| t.trf("pick.day_n", &[&n.to_string()])).collect();
                        match pick::select_index(ui, &format!("when-day-{i}"), label, n as usize - 1, &words) {
                            Some(k) => {
                                *s = (k + 1).to_string();
                                true
                            }
                            None => false,
                        }
                    })
                    .inner
                }
                None => typed(ui, label, s, 50.0, &key),
            }
        };
        let year_date = |ui: &mut egui::Ui, label: &str, s: &mut String| -> bool {
            let key = format!("date-{i}");
            match pick::month_day_parts(s).filter(|_| !typing(ui, &key)) {
                Some((m, d)) => {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let mut hit = false;
                        let months: Vec<String> = (1..=12).map(|m| t.tr(&format!("pick.month_{m}")).to_owned()).collect();
                        if let Some(k) = pick::select_index(
                            ui,
                            &format!("when-month-{i}"),
                            &format!("{label} {}", t.tr("pick.date_month")),
                            m as usize - 1,
                            &months,
                        ) {
                            *s = pick::month_day_text(k as u32 + 1, d);
                            hit = true;
                        }
                        let days: Vec<String> = (1..=pick::days_in_month(m)).map(|n| t.trf("pick.day_n", &[&n.to_string()])).collect();
                        if let Some(k) = pick::select_index(
                            ui,
                            &format!("when-monthday-{i}"),
                            &format!("{label} {}", t.tr("pick.date_day")),
                            d as usize - 1,
                            &days,
                        ) {
                            *s = pick::month_day_text(m, k as u32 + 1);
                            hit = true;
                        }
                        hit
                    })
                    .inner
                }
                None => typed(ui, label, s, 70.0, &key),
            }
        };
        // The weekdays: seven switches (and 毎日 first where an empty list means every day).
        let days = |ui: &mut egui::Ui, s: &mut String, every: bool| -> bool {
            let (label, key) = (t.tr("when.days"), format!("days-{i}"));
            if pick::days_set(s).is_some() && !typing(ui, &key) {
                named(ui, label);
                pick::day_toggles(ui, t, label, s, every.then(|| t.tr("pick.everyday")))
            } else {
                typed(ui, label, s, 160.0, &key)
            }
        };
        // A net out of the engine's list, when the text is empty or reads as one and is not being typed.
        let net = |ui: &mut egui::Ui, label: &str, s: &mut String, key: &str, optional: bool| -> bool {
            let typed_key = format!("{key}-{i}");
            if (s.trim().is_empty() || koyomi_core::jobnet::is_id(s.trim())) && !typing(ui, &typed_key) {
                ui.horizontal(|ui| {
                    ui.label(label);
                    pick.net_select(ui, t, &typed_key, label, s, optional)
                })
                .inner
            } else {
                typed(ui, label, s, 200.0, &typed_key)
            }
        };
        // A date field: typed, or picked on a month grid; `many` is a comma-separated list where a click
        // toggles a day. Each `[[when]]` has its own grid.
        let look = ito_prefs::Look { c: ito_theme::colors(), q: "" };
        let date = |ui: &mut egui::Ui, label: &str, s: &mut String, key: &str, many: bool| -> bool {
            let mut hit = false;
            ui.horizontal(|ui| {
                ui.label(label);
                let f =
                    ito_prefs::DateField::new(("when", key, i), ito_prefs::DateWords::for_lang(t.tr("app.lang"))).name(label).empty_ok();
                let f = if many { f.multi().width(260.0) } else { f.width(130.0) };
                hit = f.show(ui, look, s).changed;
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
                changed |= days(ui, &mut self.days, true);
            }
            "week" => {
                changed |= days(ui, &mut self.days, false);
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
                        changed |= month_day(ui, t.tr("when.day"), &mut self.day);
                        changed |= field(ui, t.tr("when.shift"), &mut self.shift, 50.0);
                    }
                    "nth" => {
                        changed |= nth(ui, t.tr("when.nth"), &mut self.nth);
                        changed |= weekday(ui, t.tr("when.weekday"), &mut self.weekday);
                    }
                    "business_day" => changed |= field(ui, t.tr("when.business_day"), &mut self.n, 50.0),
                    _ => {}
                }
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "year" => {
                changed |= year_date(ui, t.tr("when.date"), &mut self.date);
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "dates" => {
                changed |= date(ui, t.tr("when.dates"), &mut self.dates, "dates", true);
                changed |= field(ui, t.tr("when.at"), &mut self.at, 160.0);
            }
            "file" => {
                changed |= path(ui, t.tr("when.file"), &mut self.file, 200.0, "when.file", Want::File);
                changed |= field(ui, t.tr("when.settle"), &mut self.settle, 70.0);
            }
            "log" => {
                changed |= path(ui, t.tr("when.log"), &mut self.log, 200.0, "when.log", Want::File);
                changed |= field(ui, t.tr("when.contains"), &mut self.contains, 160.0);
            }
            "eventlog" => {
                changed |= field(ui, t.tr("when.eventlog"), &mut self.eventlog, 120.0);
                changed |= field(ui, t.tr("when.ids"), &mut self.ids, 120.0);
                changed |= field(ui, t.tr("when.source"), &mut self.source, 160.0);
            }
            "mail" => {
                if self.mailbox.trim().is_empty() {
                    changed |= field(ui, t.tr("when.mail"), &mut self.mail, 240.0);
                    changed |= field(ui, t.tr("when.password"), &mut self.password, 120.0);
                } else {
                    changed |= field(ui, t.tr("when.mailbox"), &mut self.mailbox, 160.0);
                }
                changed |= field(ui, t.tr("when.sender"), &mut self.sender, 160.0);
                changed |= field(ui, t.tr("when.subject"), &mut self.subject, 160.0);
            }
            "after_jobnet" => {
                changed |= net(ui, t.tr("when.jobnet"), &mut self.jobnet, "jobnet", false);
                ui.horizontal(|ui| {
                    ui.label(t.tr("when.on"));
                    let options = [
                        (AfterOn::Ok, t.tr("canvas.line_ok")),
                        (AfterOn::Error, t.tr("canvas.line_error")),
                        (AfterOn::Any, t.tr("canvas.line_any")),
                    ];
                    let id = format!("when-on-{i}");
                    if let (Some(on), _) = ito_prefs::seg_named(ui, look, &id, t.tr("when.on"), self.on, &options) {
                        self.on = on;
                        changed = true;
                    }
                });
            }
            "idle" => changed |= field(ui, t.tr("when.idle"), &mut self.idle, 70.0),
            "session" => {
                ui.horizontal(|ui| {
                    ui.label(t.tr("when.session"));
                    let words = Session::ALL.map(|s| t.tr(&format!("when.session_{}", s.name())).to_owned());
                    let choices: Vec<(Session, &str)> = Session::ALL.into_iter().zip(words.iter().map(String::as_str)).collect();
                    if let Some(s) = ito_prefs::select(ui, &format!("when-session-{i}"), self.session, &choices) {
                        self.session = s;
                        changed = true;
                    }
                });
                changed |= field(ui, t.tr("when.user"), &mut self.user, 160.0);
            }
            _ => {}
        }
        if !matches!(self.kind.as_str(), "on_start" | "idle" | "session" | "after_jobnet" | "file" | "log" | "eventlog" | "mail") {
            changed |= date(ui, t.tr("when.except"), &mut self.except, "except", true);
            changed |= date(ui, t.tr("when.start"), &mut self.start, "start", false);
            changed |= date(ui, t.tr("when.end"), &mut self.end, "end", false);
            changed |= net(ui, t.tr("when.wait_for"), &mut self.wait_for, "wait_for", true);
            changed |= field(ui, t.tr("when.wait_until"), &mut self.wait_until, 70.0);
            ui.horizontal(|ui| {
                ui.label(t.tr("when.on_closed"));
                // Five ways: a list that drops down, as a segmented row would not fit the pane.
                let choices = [
                    (None, t.tr("when.closed_default")),
                    (Some(OnClosed::Prev), t.tr("when.closed_prev")),
                    (Some(OnClosed::Next), t.tr("when.closed_next")),
                    (Some(OnClosed::Skip), t.tr("when.closed_skip")),
                    (Some(OnClosed::Run), t.tr("when.closed_run")),
                ];
                if let Some(c) = ito_prefs::select(ui, &format!("when-closed-{i}"), self.on_closed, &choices) {
                    self.on_closed = c;
                    changed = true;
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
            [[when]]\nidle = \"15m\"\n\
            [[when]]\nsession = \"lock\"\n\
            [[when]]\nsession = \"unlock\"\nuser = 'PC\\taro'\n\
            [[when]]\nafter_jobnet = \"経理/集計\"\non = \"error\"\n\
            [[job]]\nid = \"a\"\nrun = [\"x\"]\n";
        let net = koyomi_core::jobnet::parse(text).unwrap();
        assert_eq!(net.when.len(), 20);
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
        let mut e = WhenEdit::new("session");
        e.user = "taro@corp.example".into();
        assert!(e.to_when().unwrap_err().contains("user"));
        e.user = " PC\\taro ".into();
        assert_eq!(e.to_when().unwrap().kind, Kind::Session { session: Session::Lock, user: Some("PC\\taro".into()) });
        let mut e = WhenEdit::new("day");
        e.start = "2026/04/01".into();
        assert!(e.to_when().unwrap_err().contains(&koyomi_core::msg::DATE_BAD.code()));
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

    /// A `[[when]] mailbox` goes through the edit and back, and into the text, as it is.
    #[test]
    fn a_registered_mailbox_passes_through_the_edit() {
        let text = "format = 1
[[when]]
mailbox = \"billing\"
subject = \"Invoice\"
[[job]]
id = \"a\"
run = [\"x\"]
";
        let net = koyomi_core::jobnet::parse(text).unwrap();
        let e = WhenEdit::from_when(&net.when[0]);
        assert_eq!(e.mailbox, "billing");
        assert_eq!(e.to_when().as_ref(), Ok(&net.when[0]));
        let again = koyomi_core::jobnet::parse(&koyomi_core::write::text(&net)).unwrap();
        assert_eq!(again.when, net.when);
    }
}
