//! `koyomi mail run`: what a `mail` job runs. It reads what to send from the
//! `KOYOMI_MAIL_*` the engine sets (each after [`MAIL_TEXT`], a list one per
//! line), finds the files to attach, builds the message with ito-mail and sends
//! it through a `[[notify.mails]]` place of `koyomi.toml`, its server and its
//! login (the password from the credential store, as a notice's). It ends with 0
//! once the server took it; without, with [`NO_EXIT`](crate::ssh) and why in the
//! file `KOYOMI_REASON` names.
//!
//! The send itself cannot be asked to stop: a stop before it sends nothing, and
//! one during it ends the process tree, as the engine does any job's.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use ito_notify::mail::{address, Mail};
use koyomi_core::chore::{Chore, FILES_MAX, MAIL_TEXT};

/// The most the attached files may come to together. Many servers take 25 MB
/// in all, and the message grows by a third in base64.
const SIZE_MAX: u64 = 18 * 1024 * 1024;

/// How long the server may take to take the message.
const SEND_TIMEOUT: Duration = Duration::from_secs(600);

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => crate::ssh::run(exec),
        _ => {
            crate::say("koyomi: mail takes run (what a mail job does)", true);
            2
        }
    }
}

/// `KOYOMI_MAIL_<name>` as the engine gave it, its [`MAIL_TEXT`] taken off;
/// `None` when it is not there.
fn var(name: &str) -> Option<String> {
    std::env::var(format!("KOYOMI_MAIL_{name}")).ok().map(|v| text(&v).to_owned())
}

/// A text as the engine gives it, without the [`MAIL_TEXT`] put before it.
fn text(v: &str) -> &str {
    v.strip_prefix(MAIL_TEXT).unwrap_or(v)
}

/// A list as the engine gives it: one per line, the empty ones left out.
fn lines(v: &str) -> Vec<String> {
    v.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

/// The `[[notify.mails]]` place to send through: the one named `via`, or the first.
fn pick<'a>(places: &'a [Mail], via: Option<&str>) -> Result<&'a Mail, String> {
    if places.is_empty() {
        return Err("koyomi.toml に [[notify.mails]] がない（設定の通知のページでメールの送り先を足す）".into());
    }
    match via.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(&places[0]),
        Some(v) => places.iter().find(|p| p.name == v).ok_or_else(|| {
            let names: Vec<&str> = places.iter().map(|p| p.name.as_str()).filter(|n| !n.is_empty()).collect();
            let have =
                if names.is_empty() { "名前のあるものはない".to_owned() } else { format!("あるのは {}", names.join("・")) };
            format!("[[notify.mails]] に name = \"{v}\" がない（{have}）")
        }),
    }
}

/// Whether `m` can send: a server, a sender, and the password when it logs in.
fn ready(m: &Mail) -> Result<(), String> {
    let place = if m.name.is_empty() { format!("[[notify.mails]] の {}", m.host) } else { format!("[[notify.mails]] の {}", m.name) };
    if m.host.is_empty() {
        return Err("[[notify.mails]] の host が空（SMTP のサーバーを書く）".into());
    }
    if m.port == 0 {
        return Err(format!("{place}: port がない（465 か 587）"));
    }
    if address(&m.from).is_none() {
        return Err(format!("{place}: from がアドレスでない: {}", m.from));
    }
    let secret = m.secret.as_deref().is_some_and(|s| !s.is_empty() && !s.contains(['\r', '\n']));
    if !m.user.is_empty() && !secret {
        return Err(format!("{place}: パスワードを資格情報ストアから読めない（{}。設定の通知のページで入れ直す）", m.password));
    }
    Ok(())
}

/// The files `patterns` name, each a path or a folder and a name with `*` / `?`;
/// one that finds none is an error, as is more than [`FILES_MAX`] in all.
fn found(patterns: &[String]) -> Result<Vec<(std::path::PathBuf, u64)>, String> {
    let mut out = Vec::new();
    for p in patterns {
        let hits = crate::engine::files::find_files(p);
        if hits.is_empty() {
            return Err(format!("添付するファイルがない: {p}"));
        }
        out.extend(hits.into_iter().map(|(path, size, _)| (path, size)));
        if out.len() > FILES_MAX {
            return Err(format!("添付するファイルが {FILES_MAX} 個を超える（files の * で増えたものも数える）"));
        }
    }
    let total: u64 = out.iter().map(|(_, size)| size).sum();
    if total > SIZE_MAX {
        return Err(format!(
            "添付するファイルが大きすぎる（合わせて {} MB。{} MB まで）",
            total.div_ceil(1024 * 1024),
            SIZE_MAX / 1024 / 1024
        ));
    }
    Ok(out)
}

/// Sends the mail: 0 once the server took it, or why not. `stop` set before it
/// sends: nothing is sent.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let to = var("TO").map(|v| lines(&v)).ok_or("KOYOMI_MAIL_TO がない（koyomi mail run は kind = \"mail\" のジョブが動かす）")?;
    let cc = var("CC").map(|v| lines(&v)).unwrap_or_default();
    let subject = var("SUBJECT").unwrap_or_default().trim().to_owned();
    let body = var("BODY");
    let files = var("FILES").map(|v| lines(&v)).unwrap_or_default();
    let via = var("VIA");
    // Checked again now that each `{…}` is filled in.
    let job = Chore::Mail { to, cc, subject, body, files, via };
    job.check()?;
    let Chore::Mail { to, cc, subject, body, files, via } = job else { unreachable!() };

    let paths = crate::engine::Paths::find()?;
    let config = koyomi_core::config::Config::load(&paths.config)?;
    let (mut n, _) = crate::notify::read(&config);
    for w in n.reveal("koyomi") {
        crate::say(&format!("! {w}"), false);
    }
    let place = pick(&n.mail_places, via.as_deref())?;
    ready(place)?;

    let found = found(&files)?;
    let mut read = Vec::with_capacity(found.len());
    for (path, _) in &found {
        let bytes = std::fs::read(path).map_err(|e| format!("添付するファイルを読めない: {}: {e}", path.display()))?;
        read.push((name_of(path), bytes));
    }
    let attachments: Vec<ito_mail::Attachment> =
        read.iter().map(|(name, bytes)| ito_mail::Attachment { name, mime: None, bytes, cid: None }).collect();
    let message = ito_mail::compose(
        &ito_mail::Outgoing {
            from: &place.from,
            to: &to.join(", "),
            cc: &cc.join(", "),
            subject: &subject,
            text: body.as_deref().unwrap_or_default(),
            files: &attachments,
            ..Default::default()
        },
        SystemTime::now(),
    );
    if stop.load(Ordering::SeqCst) {
        return Err("止めた（送る前だったので、何も送っていない）".into());
    }
    let all: Vec<String> = to.iter().chain(&cc).cloned().collect();
    ito_notify::mail::send_message(place, &all, &message, SEND_TIMEOUT).map_err(|e| format!("{}: 送れなかった: {e}", place.host))?;
    for (path, size) in &found {
        crate::say(&format!("attached: {} ({size} bytes)", path.display()), false);
    }
    crate::say(&format!("mailed: {} ({} bytes, through {})", all.join(", "), message.len(), place.host), false);
    Ok(0)
}

/// What `koyomi check` says of the `mail` jobs of `net`, one line each: no
/// `[[notify.mails]]` to send through, or none with the name in `via`.
pub fn unknown(places: &[Mail], net: &koyomi_core::jobnet::Jobnet) -> Vec<String> {
    net.jobs
        .iter()
        .filter_map(|j| match &j.part {
            koyomi_core::jobnet::Part::Chore(Chore::Mail { via, .. }) => {
                pick(places, via.as_deref()).err().map(|e| format!("{}: {e}", j.id))
            }
            _ => None,
        })
        .collect()
}

/// The name an attached file has in the mail: its own.
fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str) -> Mail {
        Mail {
            name: name.into(),
            host: "smtp.example.com".into(),
            port: 587,
            user: "jobs".into(),
            password: "secret:mail".into(),
            from: "koyomi <jobs@example.com>".into(),
            to: Vec::new(),
            states: Vec::new(),
            secret: Some("pw".into()),
        }
    }

    #[test]
    fn the_engine_s_texts_lose_their_mark_and_lists_their_blank_lines() {
        assert_eq!(text("|secret:x"), "secret:x");
        assert_eq!(text("||a"), "|a");
        assert_eq!(text("plain"), "plain");
        assert_eq!(lines("a@x\n 名前 <b@x> \n\n"), ["a@x", "名前 <b@x>"]);
        assert!(lines("").is_empty());
    }

    #[test]
    fn via_picks_the_place_by_name_and_none_the_first() {
        let places = [place(""), place("ops")];
        assert_eq!(pick(&places, None).unwrap().name, "");
        assert_eq!(pick(&places, Some(" ")).unwrap().name, "");
        assert_eq!(pick(&places, Some("ops")).unwrap().name, "ops");
        let e = pick(&places, Some("経理")).unwrap_err();
        assert!(e.contains("\"経理\"") && e.contains("ops"), "{e}");
        assert!(pick(&[], None).unwrap_err().contains("[[notify.mails]] がない"));
    }

    #[test]
    fn check_tells_of_a_mail_job_with_no_place_to_send_through() {
        let net = koyomi_core::jobnet::parse(
            "format = 1\n[[job]]\nid = \"m\"\nkind = \"mail\"\nto = [\"a@x\"]\nsubject = \"s\"\nvia = \"経理\"\n\
             [[job]]\nid = \"n\"\nkind = \"mail\"\nto = [\"a@x\"]\nsubject = \"s\"\n[[job]]\nid = \"j\"\nrun = [\"x\"]\n",
        )
        .unwrap();
        let said = unknown(&[place("ops")], &net);
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("m: ") && said[0].contains("\"経理\""), "{said:?}");
        assert!(unknown(&[place("経理")], &net).is_empty());
        assert_eq!(unknown(&[], &net).len(), 2);
    }

    #[test]
    fn a_place_that_cannot_send_says_what_it_lacks() {
        assert_eq!(ready(&place("ops")), Ok(()));
        let mut open = place("ops");
        (open.user, open.secret) = (String::new(), None);
        assert_eq!(ready(&open), Ok(()));
        let mut m = place("ops");
        m.secret = None;
        assert!(ready(&m).unwrap_err().contains("secret:mail"));
        let mut m = place("ops");
        m.from = "jobs".into();
        assert!(ready(&m).unwrap_err().contains("from"));
        let mut m = place("ops");
        m.port = 0;
        assert!(ready(&m).unwrap_err().contains("port"));
        let mut m = place("ops");
        m.host.clear();
        assert!(ready(&m).unwrap_err().contains("host"));
    }

    #[test]
    fn the_files_to_attach_are_found_counted_and_weighed() {
        let dir = std::env::temp_dir().join(format!("koyomi-mail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["a.csv", "b.csv", "c.log"] {
            std::fs::write(dir.join(n), b"x,y\n").unwrap();
        }
        let pat = |p: &str| dir.join(p).display().to_string();
        let got = found(&[pat("*.csv"), pat("c.log")]).unwrap();
        assert_eq!(got.iter().map(|(p, _)| name_of(p)).collect::<Vec<_>>(), ["a.csv", "b.csv", "c.log"]);
        assert_eq!(got[0].1, 4);
        assert!(found(&[pat("none.txt")]).unwrap_err().contains("添付するファイルがない"));
        assert!(found(&vec![pat("*.csv"); FILES_MAX / 2 + 1]).unwrap_err().contains("個を超える"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
