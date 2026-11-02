//! A Windows Task Scheduler task (the XML that `schtasks /query /xml` and the
//! scheduler's 書き出し write) as a koyomi job net: its triggers as `[[when]]`,
//! its programs as jobs one after another, its working folder as `dir`.
//!
//! What has no koyomi form (an event trigger, a ComHandler action, every
//! other day …) is left out with a note, so the import says what to look at.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};
use roxmltree::{Document, Node};

use crate::bundle::{Bundle, Net};
use crate::jobnet::{Job, Jobnet, Line, Missed};
use crate::when::{Kind, MonthRule, Session, When};
use crate::write;

/// The group a task at the root of the scheduler goes into.
pub const GROUP: &str = "タスク";

/// Whether `text` is XML rather than a bundle: what the import tells them apart by.
pub fn is_xml(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').trim_start().starts_with('<')
}

/// The text of a file as read from disk: UTF-16 (with its BOM, as the
/// scheduler writes it) or UTF-8. `None` when it is neither.
pub fn decode(bytes: &[u8]) -> Option<String> {
    let units = |b: &[u8], le: bool| -> Vec<u16> {
        b.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) }).collect()
    };
    match bytes {
        [0xff, 0xfe, rest @ ..] => String::from_utf16(&units(rest, true)).ok(),
        [0xfe, 0xff, rest @ ..] => String::from_utf16(&units(rest, false)).ok(),
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8(rest.to_vec()).ok(),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

/// The task in `text` as a bundle of one job net, and the notes on what did not come over.
pub fn bundle(text: &str) -> Result<(Bundle, Vec<String>), String> {
    let (id, net, notes) = convert(text)?;
    let mut b = Bundle::new();
    b.jobnets.push(Net { id, text: write::text(&net) });
    Ok((b, notes))
}

/// The task in `text`: its id (`グループ/名前`), the net and the notes.
pub fn convert(text: &str) -> Result<(String, Jobnet, Vec<String>), String> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = Document::parse(text).map_err(|e| format!("タスクの XML が読めない: {e}"))?;
    let task = doc.root_element();
    if task.tag_name().name() != "Task" {
        return Err(format!("タスク スケジューラの XML ではない（一番外が <{}>）", task.tag_name().name()));
    }
    let mut notes = Vec::new();
    let note = |notes: &mut Vec<String>, s: String| notes.push(format!("  注意: {s}"));

    let uri = path_text(task, &["RegistrationInfo", "URI"]).unwrap_or_default();
    let (group, name) = id_of(&uri);
    let id = format!("{group}/{name}");

    let mut net = Jobnet { name: Some(name.clone()), ..Default::default() };
    let settings = child(task, "Settings");
    if settings.and_then(|s| text_of(s, "Enabled")).is_some_and(|e| e == "false") {
        note(&mut notes, "タスクは無効になっていた（取り込んだネットは確かめるまで流れない）".to_owned());
    }
    if settings.and_then(|s| text_of(s, "StartWhenAvailable")).is_some_and(|e| e == "true") {
        net.missed = Missed::Once;
    }
    if child(task, "Principals").is_some_and(|p| p.descendants().any(|n| n.has_tag_name_local("UserId"))) {
        note(&mut notes, "実行するユーザーは取り込まない（koyomi のエンジンを動かすユーザーで流れる）".to_owned());
    }

    // An idle trigger starts once the PC has been left alone as long as the task's idle setting says.
    let idle =
        settings.and_then(|s| path_text(s, &["IdleSettings", "Duration"])).and_then(|d| duration(&d)).filter(|&s| s >= 60).unwrap_or(600);
    if let Some(triggers) = child(task, "Triggers") {
        for t in triggers.children().filter(Node::is_element) {
            match trigger(t, idle) {
                Ok((whens, more)) => {
                    net.when.extend(whens);
                    for m in more {
                        note(&mut notes, m);
                    }
                }
                Err(e) => note(&mut notes, e),
            }
        }
    }
    if net.when.is_empty() {
        note(&mut notes, "時刻を 1 つも取り込めなかった（手で流すだけのネットになる）".to_owned());
    }

    let timeout = settings.and_then(|s| text_of(s, "ExecutionTimeLimit")).and_then(|d| duration(&d)).filter(|&s| s > 0 && s != 72 * 3600);
    let description = path_text(task, &["RegistrationInfo", "Description"]).filter(|d| !d.trim().is_empty());
    if let Some(actions) = child(task, "Actions") {
        for a in actions.children().filter(Node::is_element) {
            if a.tag_name().name() != "Exec" {
                note(&mut notes, format!("<{}> の動作は取り込めない（プログラムを動かす <Exec> だけ）", a.tag_name().name()));
                continue;
            }
            let Some(command) = text_of(a, "Command").filter(|c| !c.trim().is_empty()) else { continue };
            let n = net.jobs.len() + 1;
            let mut run = vec![command.trim().trim_matches('"').to_owned()];
            if let Some(args) = text_of(a, "Arguments") {
                run.extend(split_args(&args));
            }
            if run.iter().any(|r| r.contains('%')) {
                note(&mut notes, format!("job{n} の %…% は koyomi では広げない（必要なら cmd.exe /c で包む）"));
            }
            let after = net.jobs.last().map(|j: &Job| vec![Line::ok(&j.id)]).unwrap_or_default();
            net.jobs.push(Job {
                id: format!("job{n}"),
                note: if n == 1 { description.clone() } else { None },
                run,
                dir: text_of(a, "WorkingDirectory").map(|d| d.trim().trim_matches('"').to_owned()).filter(|d| !d.is_empty()),
                after,
                timeout,
                ..Default::default()
            });
        }
    }
    if net.jobs.is_empty() {
        return Err("取り込めるプログラム（<Exec> の <Command>）が無い".to_owned());
    }
    Ok((id, net, notes))
}

/// `\フォルダー\名前` as `グループ/名前`: the last folder is the group (the root: [`GROUP`]),
/// characters a file name cannot have become `_`.
fn id_of(uri: &str) -> (String, String) {
    let clean = |s: &str| -> String {
        let s: String =
            s.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/') || c.is_control() { '_' } else { c }).collect();
        let s = s.trim().trim_end_matches('.').to_owned();
        if s.is_empty() || s == "." || s == ".." {
            "_".to_owned()
        } else {
            s
        }
    };
    let parts: Vec<&str> = uri.split('\\').filter(|p| !p.trim().is_empty()).collect();
    match parts.as_slice() {
        [] => (GROUP.to_owned(), "タスク".to_owned()),
        [name] => (GROUP.to_owned(), clean(name)),
        [.., folder, name] => (clean(folder), clean(name)),
    }
}

/// One trigger as `[[when]]`s and the notes on what of it did not come over;
/// a trigger that is off is an `Err` with the note.
fn trigger(t: Node, idle: u64) -> Result<(Vec<When>, Vec<String>), String> {
    let kind = t.tag_name().name();
    if text_of(t, "Enabled").is_some_and(|e| e == "false") {
        return Err(format!("無効のトリガー（<{kind}>）は取り込まない"));
    }
    let start = text_of(t, "StartBoundary").map(|s| boundary(&s)).transpose()?;
    let end = text_of(t, "EndBoundary").map(|s| boundary(&s)).transpose()?;
    let mut notes = Vec::new();
    let at = start.map(|s| s.time().with_nanosecond(0).unwrap_or(s.time())).unwrap_or(NaiveTime::MIN);
    let kinds: Vec<Kind> = match kind {
        "TimeTrigger" => {
            let Some(s) = start else { return Err("<TimeTrigger> に開始の時刻が無い".to_owned()) };
            vec![Kind::Once { at: s.with_nanosecond(0).unwrap_or(s) }]
        }
        "BootTrigger" | "LogonTrigger" => {
            if kind == "LogonTrigger" {
                notes.push("ログオン時のトリガーは、エンジンが始まるとき（on_start）にした".to_owned());
            }
            vec![Kind::OnStart]
        }
        "IdleTrigger" => {
            notes.push(format!("アイドル時のトリガーは idle = \"{}\"（PC がその間使われなかったとき）にした", crate::duration::text(idle)));
            vec![Kind::Idle { idle }]
        }
        "CalendarTrigger" => calendar(t, at, &mut notes)?,
        "SessionStateChangeTrigger" => {
            let session = match text_of(t, "StateChange").as_deref() {
                Some("SessionLock") => Session::Lock,
                Some("SessionUnlock") => Session::Unlock,
                other => {
                    let what = other.unwrap_or_default();
                    return Err(format!("<SessionStateChangeTrigger> の「{what}」は取り込めない（ロックとロックの解除だけ）"));
                }
            };
            // The person the trigger is for, as `user`; a SID or `user@domain` is
            // not a name the engine is told, so that one only gets a note.
            let user = match text_of(t, "UserId").map(|u| u.trim().to_owned()).filter(|u| !u.is_empty()) {
                Some(u) if crate::when::valid_session_user(&u) && !u.to_ascii_uppercase().starts_with("S-1-") => Some(u),
                Some(u) => {
                    notes.push(format!(
                        "{u} に限ったトリガーだが、この書き方は読めないので、session = \"{}\" はどのユーザーのでも流れる（user = 'PC\\名前' を書き足す）",
                        session.name()
                    ));
                    None
                }
                None => None,
            };
            vec![Kind::Session { session, user }]
        }
        other => {
            return Err(format!(
                "<{other}> のトリガーは取り込めない（時刻・日・週・月・起動時・ログオン時・アイドル時・ロック時・ロック解除時だけ）"
            ));
        }
    };
    // A repetition turns each day's time into a run of times within the day.
    let kinds = match repetition(t, at, &mut notes) {
        Some((n, minutes, from, to)) => kinds
            .into_iter()
            .map(|k| {
                let days = match &k {
                    Kind::Day { .. } => Some(Vec::new()),
                    Kind::Week { days, .. } => Some(days.clone()),
                    _ => None,
                };
                match days {
                    Some(days) if minutes => Kind::Minutes { n, from, to, days },
                    Some(days) => Kind::Hours { n, from, to, days },
                    None => {
                        notes.push("繰り返しは日・週のトリガーのときだけ取り込む".to_owned());
                        k
                    }
                }
            })
            .collect(),
        None => kinds,
    };
    let today = start.map(|s| s.date());
    let whens = kinds
        .into_iter()
        .map(|kind| When {
            // A start in the past says nothing; one ahead holds the times back until then.
            start: if kind.is_event() || matches!(kind, Kind::Once { .. }) {
                None
            } else {
                today.filter(|d| *d > ito_common::now::now_local().date_naive())
            },
            end: end.map(|e| e.date()),
            kind,
            except: Vec::new(),
            on_closed: None,
            wait_for: None,
            wait_until: None,
        })
        .collect();
    Ok((whens, notes))
}

/// A `<CalendarTrigger>`'s schedule as kinds of time at `at`.
fn calendar(t: Node, at: NaiveTime, notes: &mut Vec<String>) -> Result<Vec<Kind>, String> {
    if let Some(s) = child(t, "ScheduleByDay") {
        if text_of(s, "DaysInterval").is_some_and(|n| n.trim() != "1") {
            return Err("何日かおきのトリガーは取り込めない（毎日だけ）".to_owned());
        }
        return Ok(vec![Kind::Day { at: vec![at] }]);
    }
    if let Some(s) = child(t, "ScheduleByWeek") {
        if text_of(s, "WeeksInterval").is_some_and(|n| n.trim() != "1") {
            return Err("何週かおきのトリガーは取り込めない（毎週だけ）".to_owned());
        }
        let days = weekdays(s);
        if days.is_empty() {
            return Err("週のトリガーに曜日が無い".to_owned());
        }
        return Ok(vec![Kind::Week { days, at: vec![at] }]);
    }
    if let Some(s) = child(t, "ScheduleByMonth") {
        let months = months(s);
        let days: Vec<String> = child(s, "DaysOfMonth")
            .map(|d| d.children().filter(|n| n.has_tag_name_local("Day")).filter_map(|n| n.text()).map(|t| t.trim().to_owned()).collect())
            .unwrap_or_default();
        if days.is_empty() {
            return Err("月のトリガーに日が無い".to_owned());
        }
        let mut out = Vec::new();
        for d in &days {
            let last = d.eq_ignore_ascii_case("last");
            let day: Option<u32> = d.parse().ok().filter(|n| (1..=31).contains(n));
            match (months.len() == 12, last, day) {
                (true, true, _) => out.push(Kind::Month { rule: MonthRule::Last, at: vec![at] }),
                (true, false, Some(day)) => out.push(Kind::Month { rule: MonthRule::Day { day, shift: 0 }, at: vec![at] }),
                (false, false, Some(day)) => out.extend(months.iter().map(|&month| Kind::Year { month, day, at: vec![at] })),
                (false, true, _) => notes.push("一部の月の末日は取り込めない（毎月の末日だけ）".to_owned()),
                _ => notes.push(format!("月のトリガーの日 {d} は読めない")),
            }
        }
        return Ok(out);
    }
    if let Some(s) = child(t, "ScheduleByMonthDayOfWeek") {
        if months(s).len() != 12 {
            notes.push("曜日で決める月のトリガーは、毎月として取り込んだ（一部の月だけにはしていない）".to_owned());
        }
        let weeks: Vec<String> = child(s, "Weeks")
            .map(|w| w.children().filter(|n| n.has_tag_name_local("Week")).filter_map(|n| n.text()).map(|t| t.trim().to_owned()).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for w in &weeks {
            let Some(nth) = w.parse::<u32>().ok().filter(|n| (1..=4).contains(n)) else {
                notes.push(format!("月の第 {w} 週は取り込めない（第 1〜4 週だけ）"));
                continue;
            };
            out.extend(weekdays(s).into_iter().map(|weekday| Kind::Month { rule: MonthRule::Nth { nth, weekday }, at: vec![at] }));
        }
        if out.is_empty() {
            return Err("曜日で決める月のトリガーから時刻を作れなかった".to_owned());
        }
        return Ok(out);
    }
    Err("<CalendarTrigger> の決め方が読めない".to_owned())
}

/// `<Repetition>` as (every n, in minutes?, from, to) within the day, when there is one.
fn repetition(t: Node, at: NaiveTime, notes: &mut Vec<String>) -> Option<(u32, bool, NaiveTime, NaiveTime)> {
    let r = child(t, "Repetition")?;
    let every = text_of(r, "Interval").and_then(|i| duration(&i)).filter(|&s| s >= 60)?;
    let last = NaiveTime::from_hms_opt(23, 59, 0).unwrap_or(NaiveTime::MIN);
    let to = match text_of(r, "Duration").and_then(|d| duration(&d)) {
        Some(d) if d > 0 => {
            let end = at.num_seconds_from_midnight() as u64 + d;
            if end >= 24 * 3600 {
                notes.push("日をまたぐ繰り返しは、その日の 23:59 までにした".to_owned());
                last
            } else {
                // The last repeat starts before the end, not at it.
                NaiveTime::from_num_seconds_from_midnight_opt((end.saturating_sub(60) / 60 * 60) as u32, 0).unwrap_or(last)
            }
        }
        _ => {
            notes.push("期限の無い繰り返しは、その日の 23:59 までにした".to_owned());
            last
        }
    };
    let minutes = every % 3600 != 0;
    let n = if minutes { every / 60 } else { every / 3600 };
    if minutes && every % 60 != 0 {
        notes.push("繰り返しの秒は切り捨てた".to_owned());
    }
    Some((n as u32, minutes, at, to))
}

/// The weekdays under `<DaysOfWeek>`.
fn weekdays(s: Node) -> Vec<Weekday> {
    let Some(d) = child(s, "DaysOfWeek") else { return Vec::new() };
    let all = [
        ("Monday", Weekday::Mon),
        ("Tuesday", Weekday::Tue),
        ("Wednesday", Weekday::Wed),
        ("Thursday", Weekday::Thu),
        ("Friday", Weekday::Fri),
        ("Saturday", Weekday::Sat),
        ("Sunday", Weekday::Sun),
    ];
    all.iter().filter(|(n, _)| child(d, n).is_some()).map(|(_, w)| *w).collect()
}

/// The months (1..=12) under `<Months>`; none written: all twelve.
fn months(s: Node) -> Vec<u32> {
    const NAMES: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    match child(s, "Months") {
        Some(m) => (1..=12).filter(|&i| child(m, NAMES[i as usize - 1]).is_some()).collect(),
        None => (1..=12).collect(),
    }
}

/// `2026-01-01T02:00:00`, with or without a zone or fractions (the wall clock is kept).
fn boundary(s: &str) -> Result<NaiveDateTime, String> {
    let s = s.trim();
    let wall = s.get(..19).unwrap_or(s);
    NaiveDateTime::parse_from_str(wall, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").map(|d| d.and_time(NaiveTime::MIN)))
        .map_err(|_| format!("時刻 {s} が読めない"))
}

/// An ISO 8601 duration (`PT15M`, `P1D`, `P3DT2H`) in seconds.
fn duration(s: &str) -> Option<u64> {
    let s = s.trim().strip_prefix('P')?;
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let mut total = 0u64;
    for (part, units) in [
        (date, &[('Y', 365 * 86400), ('M', 30 * 86400), ('W', 7 * 86400), ('D', 86400)][..]),
        (time, &[('H', 3600), ('M', 60), ('S', 1)][..]),
    ] {
        let mut num = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
                continue;
            }
            let (_, mult) = units.iter().find(|(u, _)| *u == c)?;
            total += (num.parse::<f64>().ok()? * *mult as f64) as u64;
            num.clear();
        }
        if !num.is_empty() {
            return None;
        }
    }
    Some(total)
}

/// A Windows command line's arguments as the C runtime splits them: spaces
/// apart, `"…"` together, `\"` a quote, `""` inside quotes a quote.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut n = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    n += 1;
                }
                if chars.peek() == Some(&'"') {
                    cur.extend(std::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        chars.next();
                        cur.push('"');
                    }
                } else {
                    cur.extend(std::iter::repeat_n('\\', n));
                }
                any = true;
            }
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    quoted = !quoted;
                }
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name_local(name))
}

fn text_of(n: Node, name: &str) -> Option<String> {
    child(n, name).map(|c| c.text().unwrap_or_default().to_owned())
}

fn path_text(n: Node, path: &[&str]) -> Option<String> {
    let mut at = n;
    for p in path {
        at = child(at, p)?;
    }
    Some(at.text().unwrap_or_default().to_owned())
}

trait Local {
    fn has_tag_name_local(&self, name: &str) -> bool;
}

impl Local for Node<'_, '_> {
    fn has_tag_name_local(&self, name: &str) -> bool {
        self.is_element() && self.tag_name().name() == name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobnet;

    const DAILY: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>夜のバックアップ</Description>
    <URI>\経理\夜間バックアップ</URI>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2020-01-01T02:30:00</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>
    </CalendarTrigger>
    <CalendarTrigger>
      <StartBoundary>2020-01-01T08:00:00</StartBoundary>
      <Repetition><Interval>PT15M</Interval><Duration>PT10H</Duration></Repetition>
      <ScheduleByWeek><DaysOfWeek><Monday /><Friday /></DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek>
    </CalendarTrigger>
    <CalendarTrigger>
      <StartBoundary>2020-01-01T23:00:00</StartBoundary>
      <ScheduleByMonth><DaysOfMonth><Day>25</Day><Day>Last</Day></DaysOfMonth>
        <Months><January/><February/><March/><April/><May/><June/><July/><August/><September/><October/><November/><December/></Months>
      </ScheduleByMonth>
    </CalendarTrigger>
    <EventTrigger><Subscription>x</Subscription></EventTrigger>
  </Triggers>
  <Principals><Principal id="Author"><UserId>S-1-5-18</UserId></Principal></Principals>
  <Settings>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT2H</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"C:\Program Files\backup\bk.exe"</Command>
      <Arguments>/to "D:\backup dir" /q</Arguments>
      <WorkingDirectory>C:\work</WorkingDirectory>
    </Exec>
    <Exec><Command>C:\tools\done.cmd</Command></Exec>
  </Actions>
</Task>
"#;

    #[test]
    fn a_daily_task_becomes_a_net_that_reads() {
        let (id, net, notes) = convert(DAILY).unwrap();
        assert_eq!(id, "経理/夜間バックアップ");
        assert_eq!(net.missed, Missed::Once);
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let kinds: Vec<&Kind> = net.when.iter().map(|w| &w.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &Kind::Day { at: vec![t(2, 30)] },
                &Kind::Minutes { n: 15, from: t(8, 0), to: t(17, 59), days: vec![Weekday::Mon, Weekday::Fri] },
                &Kind::Month { rule: MonthRule::Day { day: 25, shift: 0 }, at: vec![t(23, 0)] },
                &Kind::Month { rule: MonthRule::Last, at: vec![t(23, 0)] },
            ]
        );
        assert_eq!(net.jobs.len(), 2);
        assert_eq!(net.jobs[0].run, vec![r"C:\Program Files\backup\bk.exe", "/to", r"D:\backup dir", "/q"]);
        assert_eq!(net.jobs[0].dir.as_deref(), Some(r"C:\work"));
        assert_eq!(net.jobs[0].timeout, Some(7200));
        assert_eq!(net.jobs[0].note.as_deref(), Some("夜のバックアップ"));
        assert_eq!(net.jobs[1].after, vec![Line::ok("job1")]);
        assert!(notes.iter().any(|n| n.contains("EventTrigger")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("ユーザー")), "{notes:?}");
        // What the import writes reads back as the same net.
        let (b, _) = bundle(DAILY).unwrap();
        let back = jobnet::parse(&b.jobnets[0].text).unwrap();
        assert_eq!(back.when, net.when);
        assert_eq!(back.jobs.iter().map(|j| &j.run).collect::<Vec<_>>(), net.jobs.iter().map(|j| &j.run).collect::<Vec<_>>());
    }

    #[test]
    fn some_months_become_dates_of_the_year_and_the_rest_is_noted() {
        let xml = r#"<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <CalendarTrigger><StartBoundary>2020-01-01T09:00:00</StartBoundary>
      <ScheduleByMonth><DaysOfMonth><Day>1</Day></DaysOfMonth><Months><April/><October/></Months></ScheduleByMonth></CalendarTrigger>
    <CalendarTrigger><StartBoundary>2020-01-01T09:00:00</StartBoundary>
      <ScheduleByMonthDayOfWeek><Weeks><Week>2</Week><Week>Last</Week></Weeks><DaysOfWeek><Tuesday/></DaysOfWeek></ScheduleByMonthDayOfWeek></CalendarTrigger>
    <CalendarTrigger><StartBoundary>2020-01-01T09:00:00</StartBoundary><ScheduleByDay><DaysInterval>2</DaysInterval></ScheduleByDay></CalendarTrigger>
    <TimeTrigger><StartBoundary>2026-12-01T21:00:00+09:00</StartBoundary></TimeTrigger>
    <BootTrigger/>
  </Triggers>
  <Actions><Exec><Command>run.exe</Command></Exec><ShowMessage><Title>x</Title></ShowMessage></Actions>
</Task>"#;
        let (id, net, notes) = convert(xml).unwrap();
        assert_eq!(id, "タスク/タスク");
        let t = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        let kinds: Vec<&Kind> = net.when.iter().map(|w| &w.kind).collect();
        let once = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap().and_hms_opt(21, 0, 0).unwrap();
        assert_eq!(
            kinds,
            vec![
                &Kind::Year { month: 4, day: 1, at: vec![t] },
                &Kind::Year { month: 10, day: 1, at: vec![t] },
                &Kind::Month { rule: MonthRule::Nth { nth: 2, weekday: Weekday::Tue }, at: vec![t] },
                &Kind::Once { at: once },
                &Kind::OnStart,
            ]
        );
        assert!(notes.iter().any(|n| n.contains("第 Last 週")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("何日かおき")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("ShowMessage")), "{notes:?}");
    }

    #[test]
    fn an_idle_trigger_starts_when_the_pc_is_left_alone() {
        let settings = "<Settings><IdleSettings><Duration>PT15M</Duration></IdleSettings></Settings>";
        let xml = format!(
            r#"<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><IdleTrigger/></Triggers>
  {settings}
  <Actions><Exec><Command>run.exe</Command></Exec></Actions>
</Task>"#
        );
        let (_, net, notes) = convert(&xml).unwrap();
        assert_eq!(net.when.iter().map(|w| &w.kind).collect::<Vec<_>>(), [&Kind::Idle { idle: 900 }]);
        assert!(net.when[0].start.is_none());
        assert!(notes.iter().any(|n| n.contains("idle = \"15m\"")), "{notes:?}");
        // With no idle setting, it waits 10 minutes.
        let (_, net, _) = convert(&xml.replace(settings, "")).unwrap();
        assert_eq!(net.when[0].kind, Kind::Idle { idle: 600 });
    }

    #[test]
    fn a_lock_or_unlock_trigger_is_a_session_when() {
        let xml = |change: &str, user: &str| {
            format!(
                r#"<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <SessionStateChangeTrigger><Enabled>true</Enabled><StateChange>{change}</StateChange>{user}</SessionStateChangeTrigger>
    <TimeTrigger><StartBoundary>2026-11-30T21:00:00</StartBoundary></TimeTrigger>
  </Triggers>
  <Actions><Exec><Command>run.exe</Command></Exec></Actions>
</Task>"#
            )
        };
        let (_, net, notes) = convert(&xml("SessionLock", "")).unwrap();
        assert_eq!(net.when[0].kind, Kind::Session { session: Session::Lock, user: None });
        assert!(notes.is_empty(), "{notes:?}");
        // The person the trigger is for comes over as `user`.
        let (_, net, notes) = convert(&xml("SessionUnlock", "<UserId>PC\\taro</UserId>")).unwrap();
        assert_eq!(net.when[0].kind, Kind::Session { session: Session::Unlock, user: Some("PC\\taro".to_owned()) });
        assert!(notes.is_empty(), "{notes:?}");
        // A SID or a `user@domain` cannot be compared: anyone's, with a note.
        for id in ["S-1-5-21-1004336348-1177238915-682003330-1001", "taro@corp.example"] {
            let (_, net, notes) = convert(&xml("SessionLock", &format!("<UserId>{id}</UserId>"))).unwrap();
            assert_eq!(net.when[0].kind, Kind::Session { session: Session::Lock, user: None });
            assert!(notes.iter().any(|n| n.contains(id) && n.contains("session = \"lock\"")), "{notes:?}");
        }
        // A remote connection has no kind here: the trigger is left out with a note, the rest comes over.
        let (_, net, notes) = convert(&xml("RemoteConnect", "")).unwrap();
        assert_eq!(net.when.len(), 1);
        assert!(matches!(net.when[0].kind, Kind::Once { .. }));
        assert!(notes.iter().any(|n| n.contains("「RemoteConnect」は取り込めない")), "{notes:?}");
    }

    #[test]
    fn a_task_with_nothing_to_run_is_refused() {
        let xml = r#"<Task><Actions><ComHandler><ClassId>x</ClassId></ComHandler></Actions></Task>"#;
        assert!(convert(xml).unwrap_err().contains("<Exec>"));
        assert!(convert("<Other/>").unwrap_err().contains("<Other>"));
    }

    #[test]
    fn utf16_with_its_bom_is_read() {
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend("<Task/>".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode(&bytes).as_deref(), Some("<Task/>"));
        assert_eq!(decode("format = 1".as_bytes()).as_deref(), Some("format = 1"));
        assert!(is_xml("\u{feff}  <?xml?>") && !is_xml("format = 1"));
    }

    #[test]
    fn arguments_split_as_the_c_runtime_does() {
        assert_eq!(split_args(r#"a "b c" d\"e "f""g" \\"h i""#), vec!["a", "b c", "d\"e", "f\"g", "\\h i"]);
        assert_eq!(split_args(r#"  "" x "#), vec!["", "x"]);
        assert_eq!(split_args(r"C:\dir\ x"), vec![r"C:\dir\", "x"]);
    }

    #[test]
    fn names_a_file_cannot_have_are_cleaned() {
        assert_eq!(id_of(r"\a\b\c:d"), ("b".to_owned(), "c_d".to_owned()));
        assert_eq!(id_of(r"\名前"), (GROUP.to_owned(), "名前".to_owned()));
        assert_eq!(duration("P1DT2H30M"), Some(95400));
        assert_eq!(duration("PT15M"), Some(900));
        assert_eq!(duration("x"), None);
    }
}
