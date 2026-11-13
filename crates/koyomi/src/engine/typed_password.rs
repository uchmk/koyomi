//! Where a password typed in the settings' アカウント・接続先 PC・S3・メールボックス・データベース forms goes: into the
//! engine PC's credential store, under a secret the engine picks, with `password = "secret:<that>"` written to
//! koyomi.toml. The value is never in koyomi.toml, a message, a log or a record.

use koyomi_core::config::Config;
use koyomi_core::msg::{self, line};

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

/// How many names `<prefix><name>`, `-2`, … are tried.
const NAMES_TRIED: u32 = 100;

/// A secret name used only to build the text the reference checks read; it is never stored.
pub(super) const PLACEHOLDER: &str = "koyomi-typed-placeholder";

/// The tables of koyomi.toml whose entry takes a typed password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Account,
    Database,
    Host,
    Bucket,
    Mailbox,
}

impl Kind {
    /// The table's name, for a label such as `[[host]] web1`.
    pub(super) fn table(self) -> &'static str {
        match self {
            Kind::Account => "account",
            Kind::Database => "database",
            Kind::Host => "host",
            Kind::Bucket => "bucket",
            Kind::Mailbox => "mailbox",
        }
    }

    /// What the name of a secret the engine picks starts with.
    pub(super) fn prefix(self) -> &'static str {
        match self {
            Kind::Account => "koyomi-account-",
            Kind::Database => "koyomi-db-",
            Kind::Host => "koyomi-host-",
            Kind::Bucket => "koyomi-s3-",
            Kind::Mailbox => "koyomi-mail-",
        }
    }

    /// The secret the entry `name` of this table has for its password, as `Config::parse` reads `old_text` (a bucket's
    /// `key_id` is never it). Trimmed; `None` when there is no such entry or it names nothing.
    pub(super) fn own(self, old_text: &str, name: &str) -> Option<String> {
        let config = Config::parse(old_text).ok()?;
        let plain = |p: &Option<String>| {
            p.as_deref().map(str::trim).and_then(|p| p.strip_prefix("secret:")).map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
        };
        let found = match self {
            Kind::Account => config.account.iter().find(|e| e.name == name).and_then(|e| e.secret().map(str::to_owned)),
            Kind::Database => config.database.iter().find(|e| e.name == name).and_then(|e| e.secret().map(str::to_owned)),
            Kind::Host => config.host.iter().find(|e| e.name == name).and_then(|e| plain(&e.password)),
            Kind::Bucket => config.bucket.iter().find(|e| e.name == name).and_then(|e| plain(&e.password)),
            Kind::Mailbox => config.mailbox.iter().find(|e| e.name == name).and_then(|e| e.secret().map(str::to_owned)),
        };
        found.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    }
}

/// The entry the typed password belongs to, as the form sent it.
pub(super) struct Typed<'a> {
    pub kind: Kind,
    /// Trimmed.
    pub name: &'a str,
    /// The request's `password` field as sent (not trimmed, not filtered); it must be `None` with a typed value.
    pub password: Option<&'a str>,
    /// A SQLite `[[database]]`: it has no password.
    pub sqlite: bool,
    pub value: &'a Hidden,
}

impl Typed<'_> {
    /// `[[host]] web1`: how the messages name the entry.
    pub(super) fn label(&self) -> String {
        format!("[[{}]] {}", self.kind.table(), self.name)
    }
}

/// Which secret takes the typed password: the entry's own, when nothing else in the new koyomi.toml or in a file on this
/// PC (`elsewhere`) refers to it; else the first free `<prefix><name>`, `-2`, … (whitespace of the name as `_`).
/// `build` makes the whole new koyomi.toml for a secret name. A name is free when it is not the entry's own, nothing
/// refers to it and the store has nothing under it. Ownership is never read from the prefix.
pub(super) fn choose(
    old_text: &str,
    typed: &Typed,
    build: &dyn Fn(&str) -> Result<String, String>,
    store: &dyn SecretStore,
    elsewhere: &dyn Fn(&str) -> bool,
) -> Result<(String, Fresh), String> {
    let own = typed.kind.own(old_text, typed.name);
    // The file as it will be, with this entry's password pointing elsewhere: what else refers to a name is in it.
    let after = build(PLACEHOLDER)?;
    if let Some(own) = &own {
        // A `refers` error counts as "refers": it cannot be ruled out.
        let alone = refers(&after, own) == Ok(false);
        if alone && !elsewhere(own) {
            return Ok((own.clone(), Fresh::No));
        }
    }
    let label = typed.label();
    let base =
        format!("{}{}", typed.kind.prefix(), typed.name.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect::<String>());
    for n in 1..=NAMES_TRIED {
        let candidate = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if own.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&candidate)) {
            continue;
        }
        if refers(old_text, &candidate) != Ok(false) || refers(&after, &candidate) != Ok(false) || elsewhere(&candidate) {
            continue;
        }
        match store.get(&candidate) {
            Err(e) => return Err(line(msg::DB_SECRET_UNREADABLE, &[&label, &candidate, &e])),
            Ok(Some(_)) => {}
            Ok(None) => return Ok((candidate, Fresh::Yes)),
        }
    }
    Err(line(msg::DB_SECRET_NO_NAME, &[&label]))
}

/// The refusals that need no store: a value with a `password` (even an empty one), with a SQLite file, or one that is
/// empty, too long or holds `\0`. Only the entry's label is told.
fn check_value(typed: &Typed) -> Result<(), String> {
    if typed.password.is_some() || typed.sqlite || !value_ok(typed.value.expose()) {
        return Err(line(msg::DB_PASSWORD_VALUE_BAD, &[&typed.label()]));
    }
    Ok(())
}

/// Saves `typed` into `old_text` (the koyomi.toml as read, its base already checked by the caller): the value check,
/// the secret, the new text (`build` for the chosen name), then the store and `write`. Each refusal changes nothing; a
/// failed `write` puts the store back as it was. Returns the secret chosen. The caller holds `remote::edits()`.
pub(super) fn put(
    old_text: &str,
    typed: &Typed,
    build: &dyn Fn(&str) -> Result<String, String>,
    store: &dyn SecretStore,
    elsewhere: &dyn Fn(&str) -> bool,
    write: &mut dyn FnMut(&str) -> Result<(), String>,
) -> Result<String, String> {
    check_value(typed)?;
    let (chosen, fresh) = choose(old_text, typed, build, store, elsewhere)?;
    let new = build(&chosen)?;
    Config::parse(&new)?;
    let label = typed.label();
    let value = typed.value.expose();
    match fresh {
        Fresh::Yes => {
            store.set(&chosen, value).map_err(|e| line(msg::DB_SECRET_STORE_FAILED, &[&label, &e]))?;
            write(&new).map_err(|e| match store.delete(&chosen) {
                Ok(()) => line(msg::DB_WRITE_FAILED, &[&label, &e]),
                Err(d) => line(msg::DB_WRITE_FAILED_SECRET_LEFT, &[&label, &e, &chosen, &d]),
            })?;
        }
        Fresh::No => {
            let before = store.get(&chosen).map_err(|e| line(msg::DB_SECRET_UNREADABLE, &[&label, &chosen, &e]))?;
            store.set(&chosen, value).map_err(|e| line(msg::DB_SECRET_STORE_FAILED, &[&label, &e]))?;
            if new != old_text {
                write(&new).map_err(|e| {
                    let back = match &before {
                        Some(old) => store.set(&chosen, old),
                        None => store.delete(&chosen),
                    };
                    match back {
                        Ok(()) => line(msg::DB_WRITE_FAILED, &[&label, &e]),
                        Err(r) => line(msg::DB_WRITE_FAILED_SECRET_CHANGED, &[&label, &e, &chosen, &r]),
                    }
                })?;
            }
        }
    }
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use koyomi_core::registry::{self, AccountEntry, BucketEntry, DatabaseEntry, HostEntry, MailboxEntry};

    use super::*;

    const PW: &str = "pw-not-to-show";
    const HK: &str = "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU";
    const ALL: [Kind; 5] = [Kind::Account, Kind::Database, Kind::Host, Kind::Bucket, Kind::Mailbox];

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

    /// `old` with the entry `name` of `kind` pointing at `secret` (`None`: as written with no password where that is
    /// allowed; a host only).
    fn put_entry(kind: Kind, old: &str, name: &str, secret: Option<&str>) -> Result<String, String> {
        let pw = secret.map(|s| format!("secret:{s}"));
        match kind {
            Kind::Account => {
                registry::put_account(old, &AccountEntry { name: name.into(), user: "u".into(), password: pw.unwrap_or_default() })
            }
            Kind::Database => registry::put_database(
                old,
                &DatabaseEntry { name: name.into(), db: format!("postgres://u@h/{name}"), password: pw, tls: None },
            ),
            Kind::Host => registry::put_host(
                old,
                &HostEntry {
                    name: name.into(),
                    on: "ssh://ops@web1".into(),
                    host_key: Some(HK.into()),
                    // A key and a password cannot both be given.
                    key: pw.is_none().then(|| "C:/keys/id".to_owned()),
                    password: pw,
                    cert: None,
                    auth: None,
                },
            ),
            Kind::Bucket => registry::put_bucket(
                old,
                &BucketEntry {
                    name: name.into(),
                    url: "s3://corp-logs/app/".into(),
                    key_id: Some("AKIAEXAMPLE".into()),
                    password: pw,
                    region: Some("ap-northeast-1".into()),
                    endpoint: None,
                },
            ),
            Kind::Mailbox => registry::put_mailbox(
                old,
                &MailboxEntry {
                    name: name.into(),
                    mail: "imaps://billing@mail.example.com/INBOX".into(),
                    password: pw.unwrap_or_default(),
                },
            ),
        }
    }

    /// A koyomi.toml of `kind` entries `(name, secret)`.
    fn toml_with(kind: Kind, entries: &[(&str, Option<&str>)]) -> String {
        let mut text = String::new();
        for (name, secret) in entries {
            text = put_entry(kind, &text, name, *secret).unwrap_or_else(|e| panic!("{e}"));
        }
        text
    }

    fn typed<'a>(kind: Kind, name: &'a str, value: &'a Hidden) -> Typed<'a> {
        Typed { kind, name, password: None, sqlite: false, value }
    }

    fn run(kind: Kind, old: &str, name: &str, store: &Fake, elsewhere: &dyn Fn(&str) -> bool) -> (Result<String, String>, Option<String>) {
        let value = Hidden::new(PW.into());
        let mut written = None;
        let r = put(old, &typed(kind, name, &value), &|s| put_entry(kind, old, name, Some(s)), store, elsewhere, &mut |t| {
            written = Some(t.to_owned());
            Ok(())
        });
        (r, written)
    }

    fn choose_for(kind: Kind, old: &str, name: &str, store: &Fake, elsewhere: &dyn Fn(&str) -> bool) -> Result<(String, Fresh), String> {
        let value = Hidden::new(PW.into());
        choose(old, &typed(kind, name, &value), &|s| put_entry(kind, old, name, Some(s)), store, elsewhere)
    }

    #[test]
    fn the_prefixes_and_labels_are_per_kind() {
        let prefixes: Vec<_> = ALL.iter().map(|k| k.prefix()).collect();
        assert_eq!(prefixes, ["koyomi-account-", "koyomi-db-", "koyomi-host-", "koyomi-s3-", "koyomi-mail-"]);
        let value = Hidden::new(PW.into());
        assert_eq!(typed(Kind::Host, "web1", &value).label(), "[[host]] web1");
    }

    #[test]
    fn a_new_entry_gets_its_prefix_and_its_name() {
        for kind in ALL {
            let store = Fake::default();
            let (r, written) = run(kind, "", "main", &store, &none);
            let secret = format!("{}main", kind.prefix());
            assert_eq!(r, Ok(secret.clone()), "{kind:?}");
            assert!(written.unwrap().contains(&format!("password = \"secret:{secret}\"")), "{kind:?}");
            assert_eq!(store.items.borrow().get(&secret).map(String::as_str), Some(PW), "{kind:?}");
        }
    }

    #[test]
    fn a_name_with_a_space_gets_an_underscore() {
        let (chosen, fresh) = choose_for(Kind::Database, "", "my db", &Fake::default(), &none).unwrap();
        assert_eq!((chosen.as_str(), fresh), ("koyomi-db-my_db", Fresh::Yes));
        // `build` is only given the secret name, so a name the table would refuse is not tried here; the prefix and
        // the underscore are the module's own.
        let mut name = String::from("a");
        name.push('\u{3000}');
        name.push('b');
        let value = Hidden::new(PW.into());
        let t = typed(Kind::Mailbox, &name, &value);
        let r = choose("", &t, &|_| Ok(String::new()), &Fake::default(), &none).unwrap();
        assert_eq!(r, ("koyomi-mail-a_b".to_owned(), Fresh::Yes));
    }

    #[test]
    fn a_name_in_the_store_or_referred_to_is_skipped() {
        for kind in ALL {
            let p = kind.prefix();
            let store = Fake::with(&[(&format!("{p}a"), "x")]);
            assert_eq!(choose_for(kind, "", "a", &store, &none).unwrap().0, format!("{p}a-2"), "{kind:?}");
            let old = toml_with(kind, &[("other", Some(&format!("{p}b")))]);
            assert_eq!(choose_for(kind, &old, "b", &Fake::default(), &none).unwrap().0, format!("{p}b-2"), "{kind:?}");
            let want = format!("{p}c");
            let elsewhere = |s: &str| s == want;
            assert_eq!(choose_for(kind, "", "c", &Fake::default(), &elsewhere).unwrap().0, format!("{p}c-2"), "{kind:?}");
            let all = |_: &str| true;
            let e = choose_for(kind, "", "d", &Fake::default(), &all).unwrap_err();
            assert!(msg::DB_SECRET_NO_NAME.is_in(&e) && e.contains(&format!("[[{}]] d", kind.table())), "{kind:?}: {e}");
        }
    }

    #[test]
    fn the_entrys_own_secret_is_reused_when_only_it_refers_to_it() {
        for kind in ALL {
            let old = toml_with(kind, &[("main", Some("mine"))]);
            let store = Fake::with(&[("mine", "old-pw")]);
            let (chosen, fresh) = choose_for(kind, &old, "main", &store, &none).unwrap();
            assert_eq!((chosen.as_str(), fresh), ("mine", Fresh::No), "{kind:?}");
            // A secret that is not named with the prefix is still its own.
            assert!(store.sets_and_deletes().is_empty());
        }
    }

    #[test]
    fn another_user_of_the_secret_makes_the_entry_take_a_new_one() {
        for kind in ALL {
            let fresh_name = format!("{}main", kind.prefix());
            let one = toml_with(kind, &[("main", Some("shared"))]);
            let two = toml_with(kind, &[("main", Some("shared")), ("other", Some("shared"))]);
            let account = "[[account]]\nname = \"zz\"\nuser = \"u\"\npassword = \"secret:shared\"\n";
            let own = if kind == Kind::Account { toml_with(Kind::Mailbox, &[("m", Some("shared"))]) } else { String::new() };
            let with_other = format!("{one}{account}{own}");
            let http = format!("{one}[http]\nport = 8790\ntoken = \"secret:shared\"\n");
            for old in [two, with_other, http] {
                let store = Fake::with(&[("shared", "x")]);
                let (chosen, fresh) = choose_for(kind, &old, "main", &store, &none).unwrap_or_else(|e| panic!("{kind:?}: {e}\n{old}"));
                assert_eq!((chosen.as_str(), fresh), (fresh_name.as_str(), Fresh::Yes), "{kind:?}: {old}");
                assert!(store.sets_and_deletes().is_empty());
            }
            // A job file (elsewhere) too.
            let job = |s: &str| s == "shared";
            assert_eq!(choose_for(kind, &one, "main", &Fake::default(), &job).unwrap(), (fresh_name.clone(), Fresh::Yes), "{kind:?}");
            // A secret named with the prefix that another entry uses is not taken as the entry's own either.
            let old = toml_with(kind, &[("main", Some(&fresh_name)), ("x", Some(&fresh_name))]);
            assert_eq!(choose_for(kind, &old, "main", &Fake::default(), &none).unwrap().0, format!("{fresh_name}-2"), "{kind:?}");
        }
    }

    #[test]
    fn a_bucket_key_id_that_shares_the_password_secret_is_not_overwritten() {
        let old = "[[bucket]]\nname = \"logs\"\nurl = \"s3://corp-logs/app/\"\nkey_id = \"secret:x\"\npassword = \"secret:x\"\nregion = \"ap-northeast-1\"\n";
        let build = |s: &str| {
            registry::put_bucket(
                old,
                &BucketEntry {
                    name: "logs".into(),
                    url: "s3://corp-logs/app/".into(),
                    key_id: Some("secret:x".into()),
                    password: Some(format!("secret:{s}")),
                    region: Some("ap-northeast-1".into()),
                    endpoint: None,
                },
            )
        };
        let value = Hidden::new(PW.into());
        let store = Fake::with(&[("x", "key-id")]);
        let (chosen, fresh) = choose(old, &typed(Kind::Bucket, "logs", &value), &build, &store, &none).unwrap();
        assert_eq!((chosen.as_str(), fresh), ("koyomi-s3-logs", Fresh::Yes));
        // The key ID's secret is never the bucket's own password secret.
        assert_eq!(Kind::Bucket.own(old, "logs").as_deref(), Some("x"));
        let only_key = "[[bucket]]\nname = \"logs\"\nurl = \"s3://corp-logs/app/\"\nkey_id = \"secret:k\"\n";
        assert_eq!(Kind::Bucket.own(only_key, "logs"), None);
        assert!(store.sets_and_deletes().is_empty());
    }

    #[test]
    fn a_store_read_error_refuses_and_changes_nothing() {
        for kind in ALL {
            let store = Fake { fail_get: true, ..Fake::default() };
            let (r, written) = run(kind, "", "main", &store, &none);
            let e = r.unwrap_err();
            assert!(msg::DB_SECRET_UNREADABLE.is_in(&e) && !e.contains(PW), "{kind:?}: {e}");
            assert!(written.is_none());
            assert!(store.sets_and_deletes().is_empty());
        }
    }

    #[test]
    fn a_store_write_error_refuses_and_leaves_koyomi_toml() {
        for kind in ALL {
            let store = Fake { fail_set: true, ..Fake::default() };
            let (r, written) = run(kind, "", "main", &store, &none);
            let e = r.unwrap_err();
            assert!(msg::DB_SECRET_STORE_FAILED.is_in(&e) && !e.contains(PW), "{kind:?}: {e}");
            assert!(written.is_none());
        }
    }

    fn failing(kind: Kind, old: &str, name: &str, store: &Fake) -> String {
        let value = Hidden::new(PW.into());
        let e =
            put(old, &typed(kind, name, &value), &|s| put_entry(kind, old, name, Some(s)), store, &none, &mut |_| Err("disk full".into()))
                .unwrap_err();
        assert!(!e.contains(PW), "{e}");
        e
    }

    #[test]
    fn a_failed_write_of_a_new_secret_deletes_it() {
        for kind in ALL {
            let secret = format!("{}main", kind.prefix());
            let store = Fake::default();
            let e = failing(kind, "", "main", &store);
            assert!(msg::DB_WRITE_FAILED.is_in(&e) && e.contains(&format!("[[{}]] main", kind.table())), "{kind:?}: {e}");
            assert_eq!(store.sets_and_deletes(), [format!("set {secret}"), format!("delete {secret}")]);
            assert!(store.items.borrow().is_empty());
            let store = Fake { fail_delete: true, ..Fake::default() };
            let e = failing(kind, "", "main", &store);
            assert!(msg::DB_WRITE_FAILED_SECRET_LEFT.is_in(&e) && e.contains(&secret), "{kind:?}: {e}");
        }
    }

    #[test]
    fn a_failed_write_of_the_entrys_own_secret_puts_the_old_value_back() {
        let value = Hidden::new(PW.into());
        // Another entry changes with it, so the text changes and the write is tried.
        for kind in ALL {
            let old = toml_with(kind, &[("main", Some("mine"))]);
            let build = |s: &str| put_entry(kind, &old, "main", Some(s)).map(|t| format!("{t}# changed\n"));
            let t = typed(kind, "main", &value);
            let store = Fake::with(&[("mine", "old-pw")]);
            let e = put(&old, &t, &build, &store, &none, &mut |_| Err("disk full".into())).unwrap_err();
            assert!(msg::DB_WRITE_FAILED.is_in(&e) && !e.contains(PW), "{kind:?}: {e}");
            assert_eq!(store.items.borrow().get("mine").map(String::as_str), Some("old-pw"));
            // The old value was absent: it is deleted again.
            let store = Fake::default();
            let _ = put(&old, &t, &build, &store, &none, &mut |_| Err("disk full".into())).unwrap_err();
            assert_eq!(store.sets_and_deletes(), ["set mine", "delete mine"]);
            // The restore fails.
            let store = Fake { fail_set_after_first: true, ..Fake::with(&[("mine", "old-pw")]) };
            let e = put(&old, &t, &build, &store, &none, &mut |_| Err("disk full".into())).unwrap_err();
            assert!(msg::DB_WRITE_FAILED_SECRET_CHANGED.is_in(&e) && e.contains("mine") && !e.contains(PW), "{kind:?}: {e}");
        }
    }

    #[test]
    fn a_password_only_change_sets_the_secret_and_writes_nothing() {
        for kind in ALL {
            let old = toml_with(kind, &[("main", Some("mine"))]);
            let store = Fake::with(&[("mine", "old-pw")]);
            let value = Hidden::new(PW.into());
            let mut wrote = false;
            let r = put(&old, &typed(kind, "main", &value), &|s| put_entry(kind, &old, "main", Some(s)), &store, &none, &mut |_| {
                wrote = true;
                Ok(())
            });
            assert_eq!(r, Ok("mine".to_owned()), "{kind:?}");
            assert!(!wrote, "{kind:?}");
            assert_eq!(store.items.borrow().get("mine").map(String::as_str), Some(PW));
        }
    }

    #[test]
    fn a_value_is_stored_as_typed_not_trimmed() {
        let spaced = "  pw with spaces  ";
        let value = Hidden::new(spaced.into());
        for kind in ALL {
            let store = Fake::default();
            let secret =
                put("", &typed(kind, "main", &value), &|s| put_entry(kind, "", "main", Some(s)), &store, &none, &mut |_| Ok(())).unwrap();
            assert_eq!(store.items.borrow().get(&secret).map(String::as_str), Some(spaced), "{kind:?}");
        }
    }

    #[test]
    fn bad_values_are_refused_before_any_store_call() {
        let long = Hidden::new("x".repeat(1025));
        let nul = Hidden::new("a\0b".into());
        let empty = Hidden::new(String::new());
        let ok = Hidden::new(PW.into());
        for kind in ALL {
            let store = Fake::default();
            let sqlite = Typed { sqlite: true, ..typed(kind, "main", &ok) };
            let cases: Vec<Typed> = vec![
                sqlite,
                Typed { password: Some("secret:x"), ..typed(kind, "main", &ok) },
                Typed { password: Some(""), ..typed(kind, "main", &ok) },
                Typed { password: Some("  "), ..typed(kind, "main", &ok) },
                typed(kind, "main", &empty),
                typed(kind, "main", &long),
                typed(kind, "main", &nul),
            ];
            for t in &cases {
                let e = put("", t, &|s| put_entry(kind, "", "main", Some(s)), &store, &none, &mut |_| panic!("written")).unwrap_err();
                assert!(
                    msg::DB_PASSWORD_VALUE_BAD.is_in(&e) && e.contains(&format!("[[{}]] main", kind.table())) && !e.contains(PW),
                    "{kind:?}: {e}"
                );
            }
            assert!(store.calls.borrow().is_empty(), "{kind:?}");
        }
    }
}
