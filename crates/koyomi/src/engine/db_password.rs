//! Where a password typed in the settings' データベース form goes: into the engine PC's credential store, under a
//! secret the engine picks, with `password = "secret:<that>"` written to koyomi.toml. The value is never in koyomi.toml,
//! a message, a log or a record.

use koyomi_core::chore::SqlTls;
use koyomi_core::msg::{self, line};
use koyomi_core::registry::{self, DatabaseEntry};

use crate::proto::Hidden;
use crate::remote::{refers, SecretStore};

/// Whether the secret `choose` gave is a new one (it is set, and deleted again if koyomi.toml cannot be written) or the
/// entry's own (its old value is put back instead).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fresh {
    Yes,
    No,
}

/// The longest password taken, in characters.
pub(super) const VALUE_MAX: usize = 1024;

/// Whether a typed password can be taken: not empty, at most [`VALUE_MAX`] characters, no NUL. `DatabasePut` and
/// `DatabaseTest` share it.
pub(super) fn value_ok(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= VALUE_MAX && !value.contains('\0')
}

/// How many names `koyomi-db-<name>`, `-2`, … are tried.
const NAMES_TRIED: u32 = 100;

/// The database the typed password belongs to, as the form sent it.
pub(super) struct Typed<'a> {
    /// Trimmed.
    pub name: &'a str,
    /// Trimmed.
    pub db: &'a str,
    pub tls: Option<SqlTls>,
    /// The request's `password` (`secret:<name>`); it must be `None` with a typed value.
    pub password: Option<&'a str>,
    pub value: &'a Hidden,
}

/// Which secret takes the password of `[[database]] name`: its own, when nothing else in `old_text` or in a file on
/// this PC (`elsewhere`) refers to it; else the first free `koyomi-db-<name>`, `-2`, … (whitespace of the name as `_`).
/// A name is free when it is not the entry's own, nothing refers to it and the store has nothing under it. Ownership is
/// never read from the `koyomi-db-` prefix.
pub(super) fn choose(
    old_text: &str,
    name: &str,
    store: &dyn SecretStore,
    elsewhere: &dyn Fn(&str) -> bool,
) -> Result<(String, Fresh), String> {
    let current = koyomi_core::config::Config::parse(old_text).ok().and_then(|c| {
        c.database.iter().find(|d| d.name == name).and_then(|d| d.secret()).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    });
    if let Some(own) = &current {
        // A `refers` error counts as "refers": it cannot be ruled out.
        let alone = registry::remove_database(old_text, name).is_ok_and(|rest| !refers(&rest, own).unwrap_or(true));
        if alone && !elsewhere(own) {
            return Ok((own.clone(), Fresh::No));
        }
    }
    let base = format!("koyomi-db-{}", name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect::<String>());
    for n in 1..=NAMES_TRIED {
        let candidate = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if current.as_deref().is_some_and(|own| own.eq_ignore_ascii_case(&candidate)) {
            continue;
        }
        if refers(old_text, &candidate).unwrap_or(true) || elsewhere(&candidate) {
            continue;
        }
        match store.get(&candidate) {
            Err(e) => return Err(line(msg::DB_SECRET_UNREADABLE, &[name, &candidate, &e])),
            Ok(Some(_)) => {}
            Ok(None) => return Ok((candidate, Fresh::Yes)),
        }
    }
    Err(line(msg::DB_SECRET_NO_NAME, &[name]))
}

/// The refusals that need no store: a value with a `password`, with a SQLite file, or one that is empty, too long or
/// holds `\0`. Only the entry's name is told.
fn check_value(typed: &Typed) -> Result<(), String> {
    let sqlite = typed.db.trim().get(..9).is_some_and(|p| p.eq_ignore_ascii_case("sqlite://"));
    if typed.password.is_some() || sqlite || !value_ok(typed.value.expose()) {
        return Err(line(msg::DB_PASSWORD_VALUE_BAD, &[typed.name]));
    }
    Ok(())
}

/// Saves `typed` into `old_text` (the koyomi.toml as read, its base already checked by the caller): the value check,
/// the secret, the new text, then the store and `write`. Each refusal changes nothing; a failed `write` puts the store
/// back as it was. The caller holds `remote::edits()`.
pub(super) fn put(
    old_text: &str,
    typed: &Typed,
    store: &dyn SecretStore,
    elsewhere: &dyn Fn(&str) -> bool,
    write: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    check_value(typed)?;
    let (chosen, fresh) = choose(old_text, typed.name, store, elsewhere)?;
    let entry =
        DatabaseEntry { name: typed.name.to_owned(), db: typed.db.to_owned(), password: Some(format!("secret:{chosen}")), tls: typed.tls };
    let new = registry::put_database(old_text, &entry)?;
    koyomi_core::config::Config::parse(&new)?;
    let value = typed.value.expose();
    match fresh {
        Fresh::Yes => {
            store.set(&chosen, value).map_err(|e| line(msg::DB_SECRET_STORE_FAILED, &[typed.name, &e]))?;
            write(&new).map_err(|e| match store.delete(&chosen) {
                Ok(()) => line(msg::DB_WRITE_FAILED, &[typed.name, &e]),
                Err(d) => line(msg::DB_WRITE_FAILED_SECRET_LEFT, &[typed.name, &e, &chosen, &d]),
            })
        }
        Fresh::No => {
            let before = store.get(&chosen).map_err(|e| line(msg::DB_SECRET_UNREADABLE, &[typed.name, &chosen, &e]))?;
            store.set(&chosen, value).map_err(|e| line(msg::DB_SECRET_STORE_FAILED, &[typed.name, &e]))?;
            if new == old_text {
                return Ok(());
            }
            write(&new).map_err(|e| {
                let back = match &before {
                    Some(old) => store.set(&chosen, old),
                    None => store.delete(&chosen),
                };
                match back {
                    Ok(()) => line(msg::DB_WRITE_FAILED, &[typed.name, &e]),
                    Err(r) => line(msg::DB_WRITE_FAILED_SECRET_CHANGED, &[typed.name, &e, &chosen, &r]),
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    const PW: &str = "pw-not-to-show";

    /// A store that records its calls and can be told to fail.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub items: RefCell<BTreeMap<String, String>>,
        pub calls: RefCell<Vec<String>>,
        pub fail_get: bool,
        pub fail_set: bool,
        pub fail_delete: bool,
        /// `set` fails only for the second and later calls (a restore).
        pub fail_set_after_first: bool,
    }

    // The tests are single threaded; the store is only used through `&dyn SecretStore`.
    impl SecretStore for Fake {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            self.calls.borrow_mut().push(format!("get {name}"));
            if self.fail_get {
                return Err("store down".into());
            }
            Ok(self.items.borrow().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            let sets = self.calls.borrow().iter().filter(|c| c.starts_with("set ")).count();
            self.calls.borrow_mut().push(format!("set {name}"));
            if self.fail_set || (self.fail_set_after_first && sets >= 1) {
                return Err("store refused".into());
            }
            self.items.borrow_mut().insert(name.to_owned(), value.to_owned());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("delete {name}"));
            if self.fail_delete {
                return Err("cannot delete".into());
            }
            self.items.borrow_mut().remove(name);
            Ok(())
        }
    }

    impl Fake {
        fn with(items: &[(&str, &str)]) -> Fake {
            Fake { items: RefCell::new(items.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()), ..Fake::default() }
        }
        fn sets_and_deletes(&self) -> Vec<String> {
            self.calls.borrow().iter().filter(|c| !c.starts_with("get ")).cloned().collect()
        }
    }

    fn none(_: &str) -> bool {
        false
    }

    fn toml_with(entries: &[(&str, Option<&str>)]) -> String {
        let mut out = String::new();
        for (name, pw) in entries {
            out.push_str(&format!("[[database]]\nname = \"{name}\"\ndb = \"postgres://u@h/{name}\"\n"));
            if let Some(p) = pw {
                out.push_str(&format!("password = \"secret:{p}\"\n"));
            }
        }
        out
    }

    fn typed<'a>(name: &'a str, db: &'a str, value: &'a Hidden) -> Typed<'a> {
        Typed { name, db, tls: None, password: None, value }
    }

    fn run(old: &str, name: &str, store: &Fake, elsewhere: &dyn Fn(&str) -> bool) -> (Result<(), String>, Option<String>) {
        let value = Hidden::new(PW.into());
        let db = format!("postgres://u@h/{name}");
        let mut written = None;
        let r = put(old, &typed(name, &db, &value), store, elsewhere, &mut |t| {
            written = Some(t.to_owned());
            Ok(())
        });
        (r, written)
    }

    #[test]
    fn a_new_entry_gets_koyomi_db_and_its_name() {
        let store = Fake::default();
        let (r, written) = run("", "main", &store, &none);
        assert_eq!(r, Ok(()));
        assert!(written.unwrap().contains("password = \"secret:koyomi-db-main\""));
        assert_eq!(store.items.borrow().get("koyomi-db-main").map(String::as_str), Some(PW));
    }

    #[test]
    fn a_name_with_a_space_gets_an_underscore() {
        let (chosen, fresh) = choose("", "my db", &Fake::default(), &none).unwrap();
        assert_eq!((chosen.as_str(), fresh), ("koyomi-db-my_db", Fresh::Yes));
    }

    #[test]
    fn a_name_in_the_store_or_referred_to_is_skipped() {
        let store = Fake::with(&[("koyomi-db-a", "x")]);
        assert_eq!(choose("", "a", &store, &none).unwrap().0, "koyomi-db-a-2");
        let old = toml_with(&[("other", Some("koyomi-db-b"))]);
        assert_eq!(choose(&old, "b", &Fake::default(), &none).unwrap().0, "koyomi-db-b-2");
        let elsewhere = |s: &str| s == "koyomi-db-c";
        assert_eq!(choose("", "c", &Fake::default(), &elsewhere).unwrap().0, "koyomi-db-c-2");
        let all = |_: &str| true;
        let e = choose("", "d", &Fake::default(), &all).unwrap_err();
        assert!(msg::DB_SECRET_NO_NAME.is_in(&e), "{e}");
    }

    #[test]
    fn the_entrys_own_secret_is_reused_when_only_it_refers_to_it() {
        let old = toml_with(&[("main", Some("mine"))]);
        let store = Fake::with(&[("mine", "old-pw")]);
        let (chosen, fresh) = choose(&old, "main", &store, &none).unwrap();
        assert_eq!((chosen.as_str(), fresh), ("mine", Fresh::No));
        // A secret that is not named koyomi-db-… is still its own.
        assert!(store.sets_and_deletes().is_empty());
    }

    #[test]
    fn another_user_of_the_secret_makes_the_entry_take_a_new_one() {
        let two = toml_with(&[("main", Some("shared")), ("other", Some("shared"))]);
        let account = "[[account]]\nname = \"a\"\nuser = \"u\"\npassword = \"secret:shared\"\n";
        let own = format!("{}{account}", toml_with(&[("main", Some("shared"))]));
        let http = format!("{}[http]\ntoken = \"secret:shared\"\n", toml_with(&[("main", Some("shared"))]));
        for old in [two, own, http] {
            let store = Fake::with(&[("shared", "x")]);
            let (chosen, fresh) = choose(&old, "main", &store, &none).unwrap();
            assert_eq!((chosen.as_str(), fresh), ("koyomi-db-main", Fresh::Yes), "{old}");
            assert!(store.sets_and_deletes().is_empty());
        }
        // A job file (elsewhere) too.
        let old = toml_with(&[("main", Some("shared"))]);
        let job = |s: &str| s == "shared";
        assert_eq!(choose(&old, "main", &Fake::default(), &job).unwrap(), ("koyomi-db-main".to_owned(), Fresh::Yes));
        // A secret named koyomi-db-… that another entry uses is not taken as the entry's own either.
        let old = toml_with(&[("main", Some("koyomi-db-main")), ("x", Some("koyomi-db-main"))]);
        assert_eq!(choose(&old, "main", &Fake::default(), &none).unwrap().0, "koyomi-db-main-2");
    }

    #[test]
    fn a_store_read_error_refuses_and_changes_nothing() {
        let store = Fake { fail_get: true, ..Fake::default() };
        let (r, written) = run("", "main", &store, &none);
        let e = r.unwrap_err();
        assert!(msg::DB_SECRET_UNREADABLE.is_in(&e) && !e.contains(PW), "{e}");
        assert!(written.is_none());
        assert!(store.sets_and_deletes().is_empty());
    }

    #[test]
    fn a_store_write_error_refuses_and_leaves_koyomi_toml() {
        let store = Fake { fail_set: true, ..Fake::default() };
        let (r, written) = run("", "main", &store, &none);
        let e = r.unwrap_err();
        assert!(msg::DB_SECRET_STORE_FAILED.is_in(&e) && !e.contains(PW), "{e}");
        assert!(written.is_none());
    }

    fn failing(old: &str, name: &str, store: &Fake) -> String {
        let value = Hidden::new(PW.into());
        let db = format!("postgres://u@h/{name}");
        let e = put(old, &typed(name, &db, &value), store, &none, &mut |_| Err("disk full".into())).unwrap_err();
        assert!(!e.contains(PW), "{e}");
        e
    }

    #[test]
    fn a_failed_write_of_a_new_secret_deletes_it() {
        let store = Fake::default();
        let e = failing("", "main", &store);
        assert!(msg::DB_WRITE_FAILED.is_in(&e), "{e}");
        assert_eq!(store.sets_and_deletes(), ["set koyomi-db-main", "delete koyomi-db-main"]);
        assert!(store.items.borrow().is_empty());
        let store = Fake { fail_delete: true, ..Fake::default() };
        let e = failing("", "main", &store);
        assert!(msg::DB_WRITE_FAILED_SECRET_LEFT.is_in(&e) && e.contains("koyomi-db-main"), "{e}");
    }

    #[test]
    fn a_failed_write_of_the_entrys_own_secret_puts_the_old_value_back() {
        let old = toml_with(&[("main", Some("mine"))]);
        let store = Fake::with(&[("mine", "old-pw")]);
        // A changed db makes the text change, so the write is tried.
        let value = Hidden::new(PW.into());
        let t = Typed { name: "main", db: "postgres://u@other/main", tls: None, password: None, value: &value };
        let e = put(&old, &t, &store, &none, &mut |_| Err("disk full".into())).unwrap_err();
        assert!(msg::DB_WRITE_FAILED.is_in(&e) && !e.contains(PW), "{e}");
        assert_eq!(store.items.borrow().get("mine").map(String::as_str), Some("old-pw"));
        // The old value was absent: it is deleted again.
        let store = Fake::default();
        let t = Typed { name: "main", db: "postgres://u@other/main", tls: None, password: None, value: &value };
        let _ = put(&old, &t, &store, &none, &mut |_| Err("disk full".into())).unwrap_err();
        assert_eq!(store.sets_and_deletes(), ["set mine", "delete mine"]);
        // The restore fails.
        let store = Fake { fail_set_after_first: true, ..Fake::with(&[("mine", "old-pw")]) };
        let e = put(&old, &t, &store, &none, &mut |_| Err("disk full".into())).unwrap_err();
        assert!(msg::DB_WRITE_FAILED_SECRET_CHANGED.is_in(&e) && e.contains("mine") && !e.contains(PW), "{e}");
    }

    #[test]
    fn a_password_only_change_sets_the_secret_and_writes_nothing() {
        let old = toml_with(&[("main", Some("mine"))]);
        let store = Fake::with(&[("mine", "old-pw")]);
        let value = Hidden::new(PW.into());
        let db = "postgres://u@h/main";
        let mut wrote = false;
        let r = put(&old, &typed("main", db, &value), &store, &none, &mut |_| {
            wrote = true;
            Ok(())
        });
        assert_eq!(r, Ok(()));
        assert!(!wrote);
        assert_eq!(store.items.borrow().get("mine").map(String::as_str), Some(PW));
    }

    #[test]
    fn bad_values_are_refused_before_any_store_call() {
        let store = Fake::default();
        let long = Hidden::new("x".repeat(1025));
        let nul = Hidden::new("a\0b".into());
        let empty = Hidden::new(String::new());
        let ok = Hidden::new(PW.into());
        let cases: Vec<Typed> = vec![
            typed("main", "sqlite://C:\\a.db", &ok),
            Typed { password: Some("secret:x"), ..typed("main", "postgres://u@h/d", &ok) },
            Typed { password: Some(""), ..typed("main", "postgres://u@h/d", &ok) },
            typed("main", "postgres://u@h/d", &empty),
            typed("main", "postgres://u@h/d", &long),
            typed("main", "postgres://u@h/d", &nul),
        ];
        for t in &cases {
            let e = put("", t, &store, &none, &mut |_| panic!("written")).unwrap_err();
            assert!(msg::DB_PASSWORD_VALUE_BAD.is_in(&e) && !e.contains(PW), "{e}");
        }
        assert!(store.calls.borrow().is_empty());
    }
}
