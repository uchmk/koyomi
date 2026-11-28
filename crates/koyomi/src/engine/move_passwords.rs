//! The one-shot repair of `Request::MoveAddressPasswords`: a password written in the address of a `[[host]]`,
//! `[[mailbox]]` or `[[database]]` of koyomi.toml goes into the engine PC's credential store, and the row names it with
//! `password = "secret:<name>"`. A `[[bucket]]` address is only reported. The planning works on the raw text of the
//! file (`Config::parse` refuses exactly the files this repairs); a value is never in a line, a message or a record.

use std::collections::BTreeSet;

use koyomi_core::chore::{self, AddrPassword, NotMovable};
use koyomi_core::config::Config;
use koyomi_core::msg::{self, line, note};
use koyomi_core::registry::{self, RowEdit};

use super::typed_password::Kind;
use super::State;
use crate::proto::MoveBody;
use crate::remote::{refers, SecretStore};

/// How many names `<prefix><name>`, `-2`, … are tried for a row.
const NAMES_TRIED: u32 = 100;

/// One password to move. No `Debug`: `value` is the password.
pub(super) struct Move {
    /// `[[host]] web1`.
    pub label: String,
    /// The secret it is stored under.
    pub secret: String,
    value: String,
    /// The password as written (raw) and decoded: what a comment must not still hold.
    forms: Vec<String>,
}

/// What a move would do, made from one text of koyomi.toml.
pub(super) struct Plan {
    /// SHA-256 of the text planned on.
    pub sha256: String,
    /// The heading, one line per affected entry, then the notes. No value in it.
    pub lines: Vec<String>,
    /// At least one move, nothing that cannot be moved, and the result reads.
    pub can: bool,
    pub moves: Vec<Move>,
    /// koyomi.toml after the move (empty unless `can`).
    pub new_text: String,
    /// Every password written in an address of the file ([`registry::address_secrets`]): hidden from every message.
    pub secrets: Vec<String>,
}

/// The rows of koyomi.toml whose address is looked at: array, address key.
const ROWS: [(&str, &str, Option<Kind>); 3] =
    [("host", "on", Some(Kind::Host)), ("mailbox", "mail", Some(Kind::Mailbox)), ("database", "db", Some(Kind::Database))];

fn why(n: msg::Note) -> String {
    note(n, &[])
}

fn why_of(w: NotMovable) -> String {
    why(match w {
        NotMovable::Empty => msg::N_MOVE_WHY_EMPTY,
        NotMovable::Mask => msg::N_MOVE_WHY_MASK,
        NotMovable::Template => msg::N_MOVE_WHY_TEMPLATE,
        NotMovable::BadEscape => msg::N_MOVE_WHY_ESCAPE,
        NotMovable::Nul => msg::N_MOVE_WHY_NUL,
        NotMovable::TooLong => msg::N_MOVE_WHY_LONG,
    })
}

pub(super) fn redact(mut text: String, secrets: &[String]) -> String {
    for s in secrets {
        text = text.replace(s.as_str(), "***");
    }
    text
}

/// A name for the secret of a row: `<prefix><name>` with `-2`, … until free. Taken when anything of koyomi.toml
/// refers to it, a file on this PC may, a row of this plan has it (ASCII case ignored) or the store holds it.
fn free_name(
    kind: Kind,
    name: &str,
    old_text: &str,
    store: &dyn SecretStore,
    elsewhere: &dyn Fn(&str) -> bool,
    reserved: &BTreeSet<String>,
    secrets: &[String],
) -> Result<Option<String>, String> {
    let base = format!("{}{}", kind.prefix(), name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect::<String>());
    for n in 1..=NAMES_TRIED {
        let candidate = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        // A name that holds a password would be shown in the plan and the answer.
        if reserved.contains(&candidate.to_ascii_lowercase()) || secrets.iter().any(|s| candidate.contains(s.as_str())) {
            continue;
        }
        // The text so far differs from `old_text` only by the names this plan chose, and those are `reserved`.
        if refers(old_text, &candidate) != Ok(false) || elsewhere(&candidate) {
            continue;
        }
        match store.get(&candidate) {
            Err(e) => return Err(e),
            Ok(Some(_)) => {}
            Ok(None) => return Ok(Some(candidate)),
        }
    }
    Ok(None)
}

/// Plans the move on `text`. `Err` when it is not TOML. Reads the store (`get`) and nothing else.
pub(super) fn plan(text: &str, store: &dyn SecretStore, elsewhere: &dyn Fn(&str) -> bool) -> Result<Plan, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| line(msg::MOVE_PW_NOT_TOML, &[e.message()]))?;
    let secrets = registry::address_secrets(&table);
    let sha256 = koyomi_core::users::sha256(text);
    let mut item_lines: Vec<String> = Vec::new();
    let mut moves: Vec<Move> = Vec::new();
    let mut edits: Vec<RowEdit> = Vec::new();
    let mut stuck = false;
    let mut reserved: BTreeSet<String> = BTreeSet::new();
    for (array, key, kind) in ROWS {
        let Some(rows) = table.get(array).and_then(toml::Value::as_array) else { continue };
        for (index, row) in rows.iter().enumerate() {
            let Some(row) = row.as_table() else { continue };
            let Some(addr) = row.get(key).and_then(toml::Value::as_str) else { continue };
            let found = chore::address_password(addr);
            if matches!(found, AddrPassword::None) {
                continue;
            }
            let label = registry::row_label(array, index, row, &secrets);
            let mut refuse = |reason: String, lines: &mut Vec<String>| {
                lines.push(note(msg::N_MOVE_ITEM_NOT, &[&label, &reason]));
                stuck = true;
            };
            let value = match found {
                AddrPassword::None => continue,
                AddrPassword::NotMovable(w) => {
                    refuse(why_of(w), &mut item_lines);
                    continue;
                }
                AddrPassword::Movable(v) => v,
            };
            let has_own = match row.get("password") {
                None => false,
                Some(toml::Value::String(s)) if array == "mailbox" && s.is_empty() => false,
                Some(_) => true,
            };
            if has_own {
                refuse(why(msg::N_MOVE_WHY_HAS_PASSWORD), &mut item_lines);
                continue;
            }
            if array == "host" && row.contains_key("key") {
                refuse(why(msg::N_MOVE_WHY_KEY), &mut item_lines);
                continue;
            }
            let Some(kind) = kind else { continue };
            let name = row.get("name").and_then(toml::Value::as_str).map(str::trim).filter(|n| !n.is_empty());
            // A name that holds the password is not a name to build the secret's own from.
            let name =
                name.filter(|n| !secrets.iter().any(|s| n.contains(s.as_str()))).map_or_else(|| format!("row{}", index + 1), str::to_owned);
            let secret = match free_name(kind, &name, text, store, elsewhere, &reserved, &secrets) {
                Ok(Some(s)) => s,
                Err(_) | Ok(None) => {
                    refuse(why(msg::N_MOVE_WHY_STORE), &mut item_lines);
                    continue;
                }
            };
            reserved.insert(secret.to_ascii_lowercase());
            item_lines.push(note(msg::N_MOVE_ITEM, &[&label, &secret]));
            edits.push(RowEdit {
                array,
                index,
                set: vec![
                    (key.to_owned(), toml::Value::String(chore::without_password(addr).into_owned())),
                    ("password".to_owned(), toml::Value::String(format!("secret:{secret}"))),
                ],
            });
            let mut forms: Vec<String> = chore::password_span(addr).map(|r| addr[r].to_owned()).into_iter().collect();
            forms.push(value.clone());
            forms.dedup();
            moves.push(Move { label, secret, value, forms });
        }
    }
    // A bucket's address is only reported.
    if let Some(rows) = table.get("bucket").and_then(toml::Value::as_array) {
        for (index, row) in rows.iter().enumerate() {
            let Some(row) = row.as_table() else { continue };
            let held = ["url", "endpoint"].iter().any(|k| row.get(*k).and_then(toml::Value::as_str).is_some_and(chore::has_password));
            if held {
                let label = registry::row_label("bucket", index, row, &secrets);
                item_lines.push(note(msg::N_MOVE_ITEM_NOT, &[&label, &why(msg::N_MOVE_WHY_BUCKET)]));
                stuck = true;
            }
        }
    }
    let mut lines = Vec::new();
    if item_lines.is_empty() {
        lines.push(why(msg::N_MOVE_NOTHING));
        return Ok(Plan { sha256, lines, can: false, moves, new_text: String::new(), secrets });
    }
    lines.push(why(msg::N_MOVE_HEAD));
    lines.extend(item_lines);
    let mut can = false;
    let mut new_text = String::new();
    if !stuck && !moves.is_empty() {
        match registry::rewrite_rows(text, &edits) {
            Err(_) => lines.push(note(msg::N_MOVE_ITEM_NOT, &["koyomi.toml", &why(msg::N_MOVE_WHY_REWRITE)])),
            Ok(new) => match Config::parse(&new) {
                Err(e) => lines.push(note(msg::N_MOVE_STILL_BAD, &[&redact(e, &secrets)])),
                Ok(_) => {
                    can = true;
                    new_text = new;
                }
            },
        }
    }
    // A password also written in a comment stays after the move: the person is told (a warning, `can` is unchanged).
    if can {
        for m in &moves {
            let in_comment = new_text.lines().any(|l| l.find('#').is_some_and(|i| m.forms.iter().any(|f| l[i..].contains(f.as_str()))));
            if in_comment {
                lines.push(note(msg::N_MOVE_COMMENT, &[&m.label]));
            }
        }
    }
    Ok(Plan { sha256, lines, can, moves, new_text, secrets })
}

/// Deletes `names` from the store; the names whose `delete` failed (their value may still be there).
fn roll_back(store: &dyn SecretStore, names: &[&str]) -> Vec<String> {
    names.iter().filter(|n| store.delete(n).is_err()).map(|n| (*n).to_owned()).collect()
}

fn left_note(left: &[String]) -> String {
    if left.is_empty() {
        String::new()
    } else {
        format!("{} ", note(msg::N_MOVE_LEFT, &[&left.join(", ")]))
    }
}

/// Stores every value, then `write`s the new text. A failure of either takes back what was stored and writes nothing
/// (the store was changed first and koyomi.toml last).
pub(super) fn apply(plan: &Plan, store: &dyn SecretStore, write: &mut dyn FnMut(&str) -> Result<(), String>) -> Result<(), String> {
    let mut set: Vec<&str> = Vec::new();
    for m in &plan.moves {
        // The name was free, so it is ours whether or not the `set` got as far as storing.
        set.push(&m.secret);
        if let Err(e) = store.set(&m.secret, &m.value) {
            let left = roll_back(store, &set);
            return Err(line(msg::MOVE_PW_STORE_FAILED, &[&redact(e, &plan.secrets), &left_note(&left)]));
        }
    }
    if let Err(e) = write(&plan.new_text) {
        let left = roll_back(store, &set);
        return Err(line(msg::MOVE_PW_WRITE_FAILED, &[&redact(e, &plan.secrets), &left_note(&left)]));
    }
    Ok(())
}

fn body(sha256: String, lines: Vec<String>, can: bool, done: bool) -> String {
    MoveBody { sha256, lines, can, done }.to_toml()
}

impl State {
    /// See [`crate::proto::Request::MoveAddressPasswords`]. It records its own audit entry, only when koyomi.toml was
    /// written.
    pub(super) fn move_address_passwords(&mut self, dry_run: bool, base: Option<&str>, by: &str) -> Result<String, String> {
        self.move_address_passwords_with(dry_run, base, by, State::reload_config)
    }

    /// [`Self::move_address_passwords`] with the reload of koyomi.toml as `reload` (a test makes it fail).
    pub(super) fn move_address_passwords_with(
        &mut self,
        dry_run: bool,
        base: Option<&str>,
        by: &str,
        reload: impl FnOnce(&mut State),
    ) -> Result<String, String> {
        let _edit = crate::remote::edits();
        let text = self.users_text()?.unwrap_or_default();
        if base.is_some_and(|b| b != koyomi_core::users::sha256(&text)) {
            return Err(line(msg::CONFIG_CHANGED, &[]));
        }
        let secrets = self.secrets.clone();
        let paths = self.paths.clone();
        let plan = plan(&text, &*secrets, &|s| crate::remote::used_elsewhere(&paths, s))?;
        if dry_run || !plan.can {
            return Ok(body(plan.sha256, plan.lines, plan.can, false));
        }
        let mut skipped = false;
        apply(&plan, &*secrets, &mut |t| {
            skipped = self.users_write_noted(t)?;
            Ok(())
        })?;
        let labels: Vec<&str> = plan.moves.iter().map(|m| m.label.as_str()).collect();
        self.runner.record_op("move-address-passwords", &labels.join(", "), &labels.len().to_string(), by);
        let mut lines = plan.lines.clone();
        lines.push(note(msg::N_MOVE_DONE, &[&labels.len().to_string()]));
        if skipped {
            lines.push(why(msg::N_HISTORY_SKIPPED));
        }
        reload(self);
        // The file was written: whatever the reload said, the answer is "done" (an `Err` would read as "nothing moved").
        if let Some(e) = &self.bad_config {
            lines.push(line(msg::MOVE_PW_AFTER_WRITE, &[&redact(e.clone(), &plan.secrets)]));
        }
        Ok(body(plan.sha256, lines, true, true))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    const PW: &str = "not-the-password";

    /// A credential store in memory that records what was asked of it and can be told to fail.
    #[derive(Default)]
    struct Mem {
        items: Mutex<BTreeMap<String, String>>,
        calls: Mutex<Vec<String>>,
        fail_get: bool,
        /// `set` fails for the n-th call (from 1) and after.
        fail_set_from: Option<usize>,
        /// `delete` answers `Err` and does nothing.
        fail_delete: bool,
        /// A `set` that fails quotes the value it was given, as a real store's error might.
        echo_set: bool,
    }

    impl SecretStore for Mem {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            self.calls.lock().unwrap().push(format!("get {name}"));
            if self.fail_get {
                return Err("store down".into());
            }
            Ok(self.items.lock().unwrap().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            let sets = self.calls.lock().unwrap().iter().filter(|c| c.starts_with("set ")).count() + 1;
            self.calls.lock().unwrap().push(format!("set {name}"));
            if self.fail_set_from.is_some_and(|n| sets >= n) {
                if self.echo_set {
                    return Err(format!("store refused the value {value}"));
                }
                return Err("store refused".into());
            }
            self.items.lock().unwrap().insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("delete {name}"));
            if self.fail_delete {
                return Err("cannot delete".into());
            }
            self.items.lock().unwrap().remove(name);
            Ok(())
        }
    }

    impl Mem {
        fn writes(&self) -> Vec<String> {
            self.calls.lock().unwrap().iter().filter(|c| !c.starts_with("get ")).cloned().collect()
        }
    }

    fn none(_: &str) -> bool {
        false
    }

    fn all_rows() -> String {
        format!(
            "[[host]]\nname = \"web1\"\non = \"ssh://u:p%40ss@h\"\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\n\n[[mailbox]]\nname = \"box\"\nmail = \"imaps://me:{PW}@h\"\n\n[[database]]\nname = \"main\"\ndb = \"postgres://u:{PW}@h/x\"\n"
        )
    }

    #[test]
    fn a_plan_names_each_row_and_holds_no_password() {
        let store = Mem::default();
        let p = plan(&all_rows(), &store, &none).unwrap();
        assert!(p.can, "{:?}", p.lines);
        let all = p.lines.join("\n");
        for want in ["[[host]] web1", "koyomi-host-web1", "[[mailbox]] box", "koyomi-mail-box", "[[database]] main", "koyomi-db-main"] {
            assert!(all.contains(want), "{want}: {all}");
        }
        assert!(!all.contains(PW) && !all.contains("p%40ss") && !all.contains("p@ss"), "{all}");
        assert!(!p.new_text.contains(PW) && p.new_text.contains("password = \"secret:koyomi-host-web1\""), "{}", p.new_text);
        assert_eq!(p.moves.len(), 3);
        // `p%40ss` is stored decoded.
        assert_eq!(p.moves[0].value, "p@ss");
        assert!(store.writes().is_empty());
        Config::parse(&p.new_text).unwrap();
    }

    #[test]
    fn a_row_with_an_empty_mailbox_password_is_moved_and_a_set_one_is_not() {
        let text = format!("[[mailbox]]\nname = \"a\"\nmail = \"imaps://me:{PW}@h\"\npassword = \"\"\n");
        assert!(plan(&text, &Mem::default(), &none).unwrap().can);
        let text = format!("[[mailbox]]\nname = \"a\"\nmail = \"imaps://me:{PW}@h\"\npassword = \"secret:x\"\n");
        let p = plan(&text, &Mem::default(), &none).unwrap();
        assert!(!p.can && p.lines.join("\n").contains(&msg::note(msg::N_MOVE_WHY_HAS_PASSWORD, &[])));
        let host = format!("[[host]]\nname = \"a\"\non = \"ssh://u:{PW}@h\"\npassword = \"\"\n");
        assert!(!plan(&host, &Mem::default(), &none).unwrap().can);
    }

    #[test]
    fn each_row_that_cannot_be_moved_stops_the_plan_with_its_reason() {
        let cases = [
            (format!("[[host]]\nname = \"a\"\non = \"ssh://u:{PW}@h\"\nkey = \"k\"\n"), msg::N_MOVE_WHY_KEY),
            ("[[host]]\nname = \"a\"\non = \"ssh://u:@h\"\n".to_owned(), msg::N_MOVE_WHY_EMPTY),
            ("[[host]]\nname = \"a\"\non = \"ssh://u:***@h\"\n".to_owned(), msg::N_MOVE_WHY_MASK),
            ("[[database]]\nname = \"a\"\ndb = \"postgres://u:{param:pw}@h/x\"\n".to_owned(), msg::N_MOVE_WHY_TEMPLATE),
            ("[[host]]\nname = \"a\"\non = \"ssh://u:a%zzb@h\"\n".to_owned(), msg::N_MOVE_WHY_ESCAPE),
            ("[[host]]\nname = \"a\"\non = \"ssh://u:a%00b@h\"\n".to_owned(), msg::N_MOVE_WHY_NUL),
            (format!("[[host]]\nname = \"a\"\non = \"ssh://u:{}@h\"\n", "a".repeat(1025)), msg::N_MOVE_WHY_LONG),
            (format!("[[bucket]]\nname = \"b\"\nurl = \"sftp://u:{PW}@h\"\n"), msg::N_MOVE_WHY_BUCKET),
        ];
        for (text, n) in cases {
            let p = plan(&text, &Mem::default(), &none).unwrap();
            assert!(!p.can && p.moves.is_empty(), "{text}");
            assert!(p.lines.join("\n").contains(&msg::note(n, &[])), "{text}: {:?}", p.lines);
            assert!(!p.lines.join("\n").contains(PW));
        }
        // One bad row stops the others from moving too.
        let mixed = format!("{}[[host]]\nname = \"z\"\non = \"ssh://u:@h\"\n", all_rows());
        assert!(!plan(&mixed, &Mem::default(), &none).unwrap().can);
    }

    #[test]
    fn two_rows_that_differ_in_case_get_two_names_and_a_referred_name_is_taken() {
        let text = format!("[[host]]\nname = \"web\"\non = \"ssh://u:{PW}@h\"\n[[host]]\nname = \"Web\"\non = \"ssh://u:{PW}@i\"\n");
        let p = plan(&text, &Mem::default(), &none).unwrap();
        assert_eq!((p.moves[0].secret.as_str(), p.moves[1].secret.as_str()), ("koyomi-host-web", "koyomi-host-Web-2"));
        let refer = format!("[[host]]\nname = \"web\"\non = \"ssh://u:{PW}@h\"\n[[account]]\nname = \"a\"\nuser = \"u\"\npassword = \"secret: koyomi-host-web\"\n");
        assert_eq!(plan(&refer, &Mem::default(), &none).unwrap().moves[0].secret, "koyomi-host-web-2");
        let store = Mem::default();
        store.items.lock().unwrap().insert("koyomi-host-web".into(), "x".into());
        assert_eq!(plan(&text, &store, &none).unwrap().moves[0].secret, "koyomi-host-web-2");
        let used = |s: &str| s == "koyomi-host-web";
        assert_eq!(plan(&text, &Mem::default(), &used).unwrap().moves[0].secret, "koyomi-host-web-2");
    }

    #[test]
    fn a_store_that_cannot_be_read_moves_nothing() {
        let store = Mem { fail_get: true, ..Mem::default() };
        let p = plan(&all_rows(), &store, &none).unwrap();
        assert!(!p.can && p.moves.is_empty());
        assert!(p.lines.join("\n").contains(&msg::note(msg::N_MOVE_WHY_STORE, &[])));
        assert!(store.writes().is_empty());
    }

    #[test]
    fn what_is_not_toml_or_would_still_not_read_is_not_moved() {
        assert!(msg::MOVE_PW_NOT_TOML.is_in(&plan("not [ toml", &Mem::default(), &none).err().unwrap()));
        // Another key of the file is wrong too: the plan says so and moves nothing, and the words hold no password.
        let text = format!("{}[[host]]\nname = \"bad name?\"\non = \"ssh://\"\n", all_rows());
        let p = plan(&text, &Mem::default(), &none).unwrap();
        assert!(!p.can, "{:?}", p.lines);
        assert!(!p.lines.join("\n").contains(PW));
        let none_at_all = plan("concurrency = 2\n", &Mem::default(), &none).unwrap();
        assert!(!none_at_all.can && none_at_all.lines == [msg::note(msg::N_MOVE_NOTHING, &[])]);
        let mixed = format!("host = [{{ name = \"a\", on = \"ssh://u:{PW}@h\" }}, 7]\n");
        assert!(!plan(&mixed, &Mem::default(), &none).unwrap().can);
    }

    #[test]
    fn apply_stores_then_writes_and_a_failure_takes_it_all_back() {
        let store = Mem::default();
        let p = plan(&all_rows(), &store, &none).unwrap();
        let mut written = Vec::new();
        apply(&p, &store, &mut |t| {
            written.push(t.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(store.items.lock().unwrap().len(), 3);
        assert_eq!(store.items.lock().unwrap()["koyomi-host-web1"], "p@ss");
        assert_eq!(written, std::slice::from_ref(&p.new_text));

        // A failed write deletes every secret set.
        let store = Mem::default();
        let e = apply(&p, &store, &mut |_| Err("disk full".into())).unwrap_err();
        assert!(msg::MOVE_PW_WRITE_FAILED.is_in(&e) && !e.contains(PW), "{e}");
        assert!(store.items.lock().unwrap().is_empty());

        // A failed `set` midway deletes the earlier ones and the failing one.
        let store = Mem { fail_set_from: Some(2), ..Mem::default() };
        let mut touched = false;
        let e = apply(&p, &store, &mut |_| {
            touched = true;
            Ok(())
        })
        .unwrap_err();
        assert!(msg::MOVE_PW_STORE_FAILED.is_in(&e) && !touched, "{e}");
        assert!(store.items.lock().unwrap().is_empty());
        let deletes: Vec<_> = store.writes().into_iter().filter(|c| c.starts_with("delete ")).collect();
        assert_eq!(deletes, ["delete koyomi-host-web1", "delete koyomi-mail-box"]);

        // A `delete` that fails is named: its value may still be there.
        let store = Mem { fail_delete: true, ..Mem::default() };
        let e = apply(&p, &store, &mut |_| Err("disk full".into())).unwrap_err();
        assert!(msg::MOVE_PW_WRITE_FAILED.is_in(&e), "{e}");
        assert!(e.contains("koyomi-host-web1") && e.contains("koyomi-db-main"), "{e}");

        // A store error that quotes the value does not carry it into the reply.
        let store = Mem { fail_set_from: Some(1), echo_set: true, ..Mem::default() };
        let e = apply(&p, &store, &mut |_| Ok(())).unwrap_err();
        assert!(msg::MOVE_PW_STORE_FAILED.is_in(&e) && !e.contains(PW) && !e.contains("p@ss") && !e.contains("p%40ss"), "{e}");
        // So does a write error.
        let e = apply(&p, &Mem::default(), &mut |_| Err(format!("cannot write {PW}"))).unwrap_err();
        assert!(msg::MOVE_PW_WRITE_FAILED.is_in(&e) && !e.contains(PW), "{e}");
    }

    #[test]
    fn a_name_that_holds_the_password_is_not_shown_or_used() {
        let text = format!(
            "[[host]]\nname = \"{PW}\"\non = \"ssh://u:{PW}@h\"\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\n"
        );
        let p = plan(&text, &Mem::default(), &none).unwrap();
        assert!(p.can, "{:?}", p.lines);
        let all = p.lines.join("\n");
        assert!(!all.contains(PW) && all.contains("[[host]] #1"), "{all}");
        assert_eq!(p.moves[0].secret, "koyomi-host-row1");
        assert!(!p.moves[0].label.contains(PW));
    }

    #[test]
    fn a_secret_name_that_would_hold_the_password_is_skipped() {
        let text =
            "[[host]]\nname = \"web\"\non = \"ssh://u:web-2@h\"\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\n";
        let store = Mem::default();
        store.items.lock().unwrap().insert("koyomi-host-web".to_owned(), "x".to_owned());
        let p = plan(text, &store, &none).unwrap();
        assert!(p.can, "{:?}", p.lines);
        assert_eq!(p.moves[0].secret, "koyomi-host-web-3");
        assert!(!p.lines.join("\n").contains("web-2"), "{:?}", p.lines);
    }

    #[test]
    fn a_password_in_a_comment_is_warned_about_and_never_repeated() {
        let text = format!("[[host]]\nname = \"a\"\non = \"ssh://u:{PW}@h\" # was ssh://u:{PW}@h\nhost_key = \"SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU\"\n");
        let p = plan(&text, &Mem::default(), &none).unwrap();
        assert!(p.can, "{:?}", p.lines);
        let all = p.lines.join("\n");
        assert!(all.contains(&msg::note(msg::N_MOVE_COMMENT, &["[[host]] a"])), "{all}");
        assert!(!all.contains(PW), "{all}");
    }

    #[test]
    fn an_error_that_quotes_a_password_with_a_quote_is_hidden_in_both_forms() {
        let text = "concurrency = 'ssh://u:a\"b@h'\n[[host]]\nname = \"a\"\non = 'ssh://u:a\"b@h'\n";
        let p = plan(text, &Mem::default(), &none).unwrap();
        let all = p.lines.join("\n");
        assert!(!p.can && all.contains(msg::note(msg::N_MOVE_STILL_BAD, &[""]).trim()), "{all}");
        assert!(!all.contains("a\"b") && !all.contains("a\\\"b"), "{all}");
    }
}
