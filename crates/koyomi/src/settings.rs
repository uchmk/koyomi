//! The settings screen (Ctrl+,): `ito-prefs` draws the nav and the cards, and
//! the rows common to every uchmk app (the language, the size, the clock,
//! the window, the theme) write common.toml. koyomi's own pages are the
//! Notifications page (`[notify]` of koyomi.toml), the General page's
//! Records section (koyomi.toml's `keep_days`, `concurrency` and
//! `log_max_mb`; shortening `keep_days` asks first, with how many days'
//! records would go) and the Keys page (each operation's keys, written to
//! keymap.toml). The definitions are not settings: they stay files, as
//! DECISIONS.md has it.

use egui::{vec2, Align2, FontId, Sense, Stroke, StrokeKind, Ui};
use ito_common::{themes_dir, CommonChange};
use ito_prefs::{section, MoreWords, Words};

use crate::ui::RegistryPage;
use crate::App;

pub const GENERAL: usize = 0;
pub const THEME: usize = 1;
pub const NOTIFY: usize = 2;
pub const USERS: usize = 3;
pub const ACCOUNTS: usize = 4;
pub const DATABASES: usize = 5;
pub const HOSTS: usize = 6;
pub const BUCKETS: usize = 7;
pub const MAILBOXES: usize = 8;
pub const KEYS: usize = 9;
pub const ADVANCED: usize = 10;
pub const ABOUT: usize = 11;

/// The keymap's layers and the words that name them on the Keys page.
const LAYERS: [(&str, &str); 1] = [("app", "keys.main")];

/// What common.toml means when it names no theme: ito's default (`dark`,
/// tsumugi Dark and tsumugi Light, as the common spec says). koyomi has no
/// themes of its own, and no older setting to carry over.
pub const THEMES: Option<(&str, &str, &str)> = None;

/// Whether a row of the page is the engine's (its files, on its PC) and not this window's. On another PC's
/// engine these are not shown, so the search must not find them either.
fn engine_row(page: usize, text: &str, t: &ito_i18n::Table, more: &MoreWords) -> bool {
    let records = [
        t.tr("settings.records"),
        t.tr("settings.keep_days"),
        t.tr("settings.archive"),
        t.tr("settings.concurrency"),
        t.tr("settings.log_max"),
        t.tr("settings.jobs"),
        t.tr("settings.shell"),
        t.tr("settings.shell_note"),
        "keep_days",
        "archive",
        "concurrency",
        "log_max_mb",
        "shell",
    ];
    match page {
        // The Records section, and the button for koyomi.toml.
        GENERAL => text == CONFIG_FILE || records.contains(&text),
        // The whole page is `[notify]` of koyomi.toml, but for the watch switch: that is this window's own, on this
        // PC's koyomi.toml.
        NOTIFY => !(text == t.tr("settings.watch_section") || text == t.tr("settings.watch_remotes") || text == "watch_remotes"),
        // The state folder's row is this PC's engine's: ito's own card leaves it out on another PC's engine.
        ADVANCED => text == more.state_dir || text == more.state_dir_note,
        _ => false,
    }
}

/// The rows on each page, for the search. `far`: the window shows another PC's engine, so the engine's rows
/// (see [`engine_row`]) are left out.
fn index(w: &'static Words, more: &'static MoreWords, lang: &str, t: &ito_i18n::Table, far: bool) -> Vec<(usize, String)> {
    let keys = ito_prefs::keys_index(ito_prefs::KeyWords::of(lang)).into_iter().map(|t| (KEYS, t));
    let layers = LAYERS.iter().map(|(_, k)| (KEYS, t.tr(k)));
    vec![
        (GENERAL, w.language),
        (GENERAL, w.scale),
        (GENERAL, w.clock),
        (GENERAL, w.show_time),
        (GENERAL, w.time_format),
        (GENERAL, w.show_date),
        (GENERAL, w.date_format),
        (GENERAL, w.weekday),
        (GENERAL, "language"),
        (GENERAL, "scale"),
        (GENERAL, "[clock]"),
        (GENERAL, "[window]"),
        (GENERAL, t.tr("settings.records")),
        (GENERAL, t.tr("settings.keep_days")),
        (GENERAL, t.tr("settings.archive")),
        (GENERAL, t.tr("settings.concurrency")),
        (GENERAL, t.tr("settings.log_max")),
        (GENERAL, t.tr("settings.jobs")),
        (GENERAL, t.tr("settings.shell")),
        (GENERAL, "keep_days"),
        (GENERAL, "archive"),
        (GENERAL, "concurrency"),
        (GENERAL, "log_max_mb"),
        (GENERAL, "shell"),
        (GENERAL, "koyomi.toml"),
        (THEME, w.mode),
        (THEME, w.themes),
        (THEME, w.preview),
        (THEME, "theme"),
        (THEME, "dark_theme"),
        (THEME, "light_theme"),
        (NOTIFY, "[notify]"),
        (NOTIFY, "koyomi.toml"),
        (NOTIFY, t.tr("settings.watch_section")),
        (NOTIFY, t.tr("settings.watch_remotes")),
        (NOTIFY, "watch_remotes"),
        (USERS, "[[http.user]]"),
        (USERS, "koyomi.toml"),
        (USERS, t.tr("settings.users")),
        (USERS, t.tr("users.add")),
        (USERS, t.tr("users.edit")),
        (USERS, t.tr("users.reset")),
        (USERS, t.tr("users.remove")),
        (ACCOUNTS, "[[account]]"),
        (ACCOUNTS, "koyomi.toml"),
        (ACCOUNTS, t.tr("settings.accounts")),
        (ACCOUNTS, t.tr("registry.add_account")),
        (ACCOUNTS, t.tr("registry.user_field")),
        (ACCOUNTS, t.tr("registry.secret_field")),
        (ACCOUNTS, "run_as"),
        (DATABASES, "[[database]]"),
        (DATABASES, "koyomi.toml"),
        (DATABASES, t.tr("settings.databases")),
        (DATABASES, t.tr("registry.add_database")),
        (DATABASES, t.tr("registry.db_field")),
        (DATABASES, t.tr("registry.secret_field")),
        (DATABASES, t.tr("registry.tls_field")),
        (DATABASES, "database"),
        (HOSTS, "[[host]]"),
        (HOSTS, "koyomi.toml"),
        (HOSTS, t.tr("settings.hosts")),
        (HOSTS, t.tr("registry.add_host")),
        (HOSTS, t.tr("registry.host_kind")),
        (HOSTS, t.tr("registry.host_on")),
        (HOSTS, t.tr("registry.host_key_field")),
        (HOSTS, t.tr("registry.key_file")),
        (HOSTS, t.tr("registry.cert_field")),
        (HOSTS, t.tr("registry.auth_field")),
        (HOSTS, t.tr("registry.secret_field")),
        (HOSTS, "host_key"),
        (BUCKETS, "[[bucket]]"),
        (BUCKETS, "koyomi.toml"),
        (BUCKETS, t.tr("settings.buckets")),
        (BUCKETS, t.tr("registry.add_bucket")),
        (BUCKETS, t.tr("registry.bucket_url")),
        (BUCKETS, t.tr("registry.key_id_field")),
        (BUCKETS, t.tr("registry.region_field")),
        (BUCKETS, t.tr("registry.endpoint_field")),
        (BUCKETS, t.tr("registry.secret_field")),
        (BUCKETS, "S3"),
        (MAILBOXES, "[[mailbox]]"),
        (MAILBOXES, "koyomi.toml"),
        (MAILBOXES, t.tr("settings.mailboxes")),
        (MAILBOXES, t.tr("registry.add_mailbox")),
        (MAILBOXES, t.tr("registry.mailbox_mail")),
        (MAILBOXES, t.tr("registry.secret_field")),
        (MAILBOXES, "mailbox"),
        (KEYS, "keymap.toml"),
        (KEYS, "prepend_keymap"),
    ]
    .into_iter()
    .chain(ito_prefs::notify_index(w).into_iter().map(|t| (NOTIFY, t)))
    .chain(keys)
    .chain(layers)
    .chain(ito_prefs::window_index(more).into_iter().chain(ito_prefs::startup_index(more)).map(|t| (GENERAL, t)))
    .chain(ito_prefs::advanced_index(more).into_iter().map(|t| (ADVANCED, t)))
    .chain(ito_prefs::mono_font_index(more).into_iter().map(|t| (THEME, t)))
    .chain(ito_prefs::about_index(more, w).into_iter().map(|t| (ABOUT, t)))
    .filter(|(p, text)| !(far && engine_row(*p, text, t, more)))
    .map(|(p, t)| (p, t.to_owned()))
    .collect()
}

/// The one line shown where the engine's settings would be, on another PC's engine.
fn far_line(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).color(ito_theme::colors().dim)).wrap());
}

/// The 通知 page's section for what the window watches: one switch, `watch_remotes` of koyomi.toml. `pick` is what
/// the switch was turned to.
fn watch_section(ui: &mut Ui, l: ito_prefs::Look, tr: &dyn Fn(&str) -> String, on: bool, pick: &mut Option<bool>) {
    section(ui, l, &tr("settings.watch_section"), |ui| {
        let (label, note) = (tr("settings.watch_remotes"), tr("settings.watch_remotes_note"));
        if ito_prefs::keyed_row(ui, l, "watch_remotes", &label, &note, |ui| ito_prefs::switch(ui, l, on)) {
            *pick = Some(!on);
        }
    });
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let w = Words::of(&app.lang);
    let more = MoreWords::of(&app.lang);
    poll_startup(ui, app);
    let no_folder = app.table.tr("settings.no_folder").to_owned();
    // A box raised on this PC does not survive a switch to another PC's engine.
    if app.browser.link().is_far() {
        app.keep_ask = None;
    }
    keep_ask(ui, app, &no_folder);
    // While a key is being set, or the box asks, the page takes every key, Esc included.
    // A token on the screen, or a question, form or editor of the users page, takes Esc before the settings do.
    let users_keys = app.browser.holds_token()
        || (app.prefs.state.page == USERS && app.browser.users_wants_esc())
        || (app.prefs.state.page == ACCOUNTS && app.browser.registry_wants_esc(RegistryPage::Accounts))
        || (app.prefs.state.page == DATABASES && app.browser.registry_wants_esc(RegistryPage::Databases))
        || (app.prefs.state.page == HOSTS && app.browser.registry_wants_esc(RegistryPage::Hosts))
        || (app.prefs.state.page == BUCKETS && app.browser.registry_wants_esc(RegistryPage::Buckets))
        || (app.prefs.state.page == MAILBOXES && app.browser.registry_wants_esc(RegistryPage::Mailboxes));
    let keys = if app.prefs.keys_edit.busy() || app.keep_ask.is_some() || app.browser.config_open() || users_keys {
        ito_prefs::Keys::default()
    } else {
        ito_prefs::Keys::standard(ui, false)
    };
    let table = &app.table;
    let tr = |k: &str| table.tr(k).to_owned();
    let (adv, about) = (ito_prefs::Page::Advanced.words(more), ito_prefs::Page::About.words(more));
    let page_words = [
        (tr("settings.general"), tr("settings.general_lead")),
        (tr("settings.theme"), tr("settings.theme_lead")),
        (tr("settings.notify"), tr("settings.notify_lead")),
        (tr("settings.users"), tr("settings.users_lead")),
        (tr("settings.accounts"), tr("settings.accounts_lead")),
        (tr("settings.databases"), tr("settings.databases_lead")),
        (tr("settings.hosts"), tr("settings.hosts_lead")),
        (tr("settings.buckets"), tr("settings.buckets_lead")),
        (tr("settings.mailboxes"), tr("settings.mailboxes_lead")),
        (tr("settings.keys"), tr("settings.keys_lead")),
        (adv.0.to_owned(), adv.1.to_owned()),
        (about.0.to_owned(), about.1.to_owned()),
    ];
    let pages: Vec<(&str, &str)> = page_words.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let far = app.browser.link().is_far();
    let far_text = tr("settings.far_line");
    // On another PC's engine an `admin` edits its koyomi.toml in a box; the other roles are told why not.
    let (far_admin, far_not_admin) = (app.browser.far_admin(), app.browser.far_not_admin());
    let (far_config_words, far_only_words) = (tr("settings.far_config"), tr("settings.far_admin_only"));
    let mut open_config = false;
    let index = index(w, more, &app.lang, table, far);
    let index: Vec<(usize, &str)> = index.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let nav = ito_prefs::Nav { pages: &pages, index: &index, words: w, file: "common.toml" };
    let mut asked: Vec<CommonChange> = Vec::new();
    let layers = app.keys.layers().to_vec();
    let kw = ito_prefs::KeyWords::of(&app.lang);
    let own =
        app.prefs.base.as_deref().map_or_else(|| crate::keymap::FILE.to_owned(), |b| crate::prefs::keymap_path(b).display().to_string());
    let mut key_edits = Vec::new();
    // The Notifications page as it is set now, when it was changed; and
    // what "Send a test" asked for.
    let mut notify_new: Option<ito_prefs::Notify> = None;
    let mut test = None;
    // The mail places as the page has them, when changed; the one to test.
    let mut mails_new = None;
    let mut mail_test = None;
    let prefs = &mut app.prefs;
    let keys_edit = &mut prefs.keys_edit;
    let (notify, notify_warn, drafts) = (&prefs.notify, &prefs.notify_warn, &mut prefs.notify_drafts);
    // The Records section: the days picked, and the numbers typed (as typed).
    let (config, config_warn, limit_drafts) = (&prefs.config, &prefs.config_warn, &mut prefs.limit_drafts);
    let mut keep_pick: Option<u32> = None;
    let mut archive_pick: Option<bool> = None;
    // The Jobs section: the default shell picked among the ones this engine has.
    let mut shell_pick: Option<String> = None;
    let (shell_now, shell_words) = shell_choices(app.browser.shells(), config.shell.as_deref(), table);
    let shell_path =
        shell_words.get(shell_now).and_then(|(n, _)| app.browser.shells().0.iter().find(|s| s.name == *n)).map(|s| s.path.clone());
    let shell_opts: Vec<(usize, &str)> = shell_words.iter().enumerate().map(|(i, (_, w))| (i, w.as_str())).collect();
    // The watch switch of the Notifications page: this window's, whichever PC's engine it shows.
    let mut watch_pick: Option<bool> = None;
    let watch_remotes = config.watch_remotes;
    let mut limit_typed: Vec<(&'static str, String)> = Vec::new();
    let mut day_words: Vec<(u32, String)> = KEEP_DAYS.iter().map(|d| (*d, table.trf("settings.days", &[&d.to_string()]))).collect();
    if !KEEP_DAYS.contains(&config.keep_days) {
        // Written by hand: shown as it is, among the choices.
        day_words.push((config.keep_days, table.trf("settings.days", &[&config.keep_days.to_string()])));
        day_words.sort();
    }
    let day_opts: Vec<(u32, &str)> = day_words.iter().map(|(d, s)| (*d, s.as_str())).collect();
    let mut bug = false;
    let mut forget = false;
    let mut startup_toggled = None;
    let mut advanced = Vec::new();
    let startup_now = app.startup;
    let startup_problem = app.startup_problem.clone();
    let state_dir = ito_common::state_dir("koyomi");
    let (common, themes, base) = (&prefs.common, &prefs.themes, prefs.base.as_deref());
    // koyomi.toml's button above common.toml's: the engine's settings are a file of their own.
    // On another PC's engine there is no koyomi.toml of that engine to open from here.
    // While a new token is on the screen, only its box answers.
    if app.browser.holds_token() {
        ui.disable();
    }
    let extras = ito_prefs::Extras { files: if far { &[] } else { &[CONFIG_FILE] }, ..Default::default() };
    let out = ito_prefs::show_with(ui, ito_theme::colors(), &mut prefs.state, &nav, keys, extras, |ui, page, l| match page {
        GENERAL => {
            section(ui, l, &tr("settings.display"), |ui| {
                let now = common.language.as_deref().unwrap_or("auto");
                asked.extend(ito_prefs::language_row(ui, l, w, now, ""));
                ito_prefs::sep(ui, l);
                asked.extend(ito_prefs::scale_row(ui, l, w, common.scale_or_one(), ""));
            });
            asked.extend(ito_prefs::clock_card(ui, l, w, &common.clock_or(None)));
            let out = ito_prefs::window_card(ui, l, more, &common.window_or_default());
            asked.extend(out.changes);
            forget |= out.forget;
            startup_toggled = ito_prefs::startup_card_with(ui, l, more, startup_now, startup_problem.as_deref());
            if far {
                if far_admin {
                    let r = ito_prefs::IconButton::new(ito_prefs::Mark::Pencil, &far_config_words).show(ui);
                    ito_a11y::test_id(&r, "far_config.open");
                    open_config |= r.clicked();
                } else if far_not_admin {
                    far_line(ui, &far_only_words);
                } else {
                    far_line(ui, &far_text);
                }
                return;
            }
            section(ui, l, &tr("settings.records"), |ui| {
                if let Some(w) = config_warn {
                    ito_prefs::status(ui, w, l.c.err);
                }
                let (label, note) = (tr("settings.keep_days"), tr("settings.keep_days_note"));
                keep_pick = ito_prefs::keyed_row(ui, l, "keep_days", &label, &note, |ui| {
                    ito_prefs::select(ui, "keep_days", config.keep_days, &day_opts)
                });
                ito_prefs::sep(ui, l);
                let (label, note) = (tr("settings.archive"), tr("settings.archive_note"));
                if ito_prefs::keyed_row(ui, l, "archive", &label, &note, |ui| ito_prefs::switch(ui, l, config.archive)) {
                    archive_pick = Some(!config.archive);
                }
                ito_prefs::sep(ui, l);
                let (label, note) = (tr("settings.concurrency"), tr("settings.concurrency_note"));
                let now = config.concurrency.to_string();
                if let Some(t) = ito_prefs::keyed_row(ui, l, "concurrency", &label, &note, |ui| {
                    ito_prefs::field(ui, limit_drafts, "concurrency", &now, "4", 80.0)
                }) {
                    limit_typed.push(("concurrency", t));
                }
                ito_prefs::sep(ui, l);
                let (label, note) = (tr("settings.log_max"), tr("settings.log_max_note"));
                let now = config.log_max_mb.to_string();
                if let Some(t) = ito_prefs::keyed_row(ui, l, "log_max_mb", &label, &note, |ui| {
                    ito_prefs::field(ui, limit_drafts, "log_max_mb", &now, "10", 80.0)
                }) {
                    limit_typed.push(("log_max_mb", t));
                }
            });
            section(ui, l, &tr("settings.jobs"), |ui| {
                let (label, mut note) = (tr("settings.shell"), tr("settings.shell_note"));
                if let Some(p) = &shell_path {
                    note = format!("{note}\n{p}");
                }
                if let Some(i) =
                    ito_prefs::keyed_row(ui, l, "shell", &label, &note, |ui| ito_prefs::select(ui, "shell", shell_now, &shell_opts))
                {
                    shell_pick = shell_words.get(i).map(|(n, _)| n.clone());
                }
            });
        }
        THEME => {
            let (t, d, li) = common.theme_choice(THEMES);
            let note = match base {
                Some(b) => format!("{} {}", tr("settings.theme_note"), themes_dir(b).display()),
                None => tr("settings.no_folder"),
            };
            let runs = [tr("state.ok"), tr("state.error"), tr("state.running")];
            asked.extend(ito_prefs::theme_page(ui, l, w, themes, (&t, &d, &li), &note, |ui| preview(ui, &tr("settings.sample"), &runs)));
            // Read at the next start: the fonts are loaded once, in the background.
            asked.extend(ito_prefs::mono_font_card(ui, l, more, common.mono_font_or_default()));
        }
        NOTIFY if far => {
            watch_section(ui, l, &tr, watch_remotes, &mut watch_pick);
            far_line(ui, &far_text);
        }
        NOTIFY => {
            watch_section(ui, l, &tr, watch_remotes, &mut watch_pick);
            for n in notify_warn {
                ito_prefs::status(ui, n, l.c.err);
            }
            let mut n = notify.page(&crate::notify::EVENTS);
            let was = n.clone();
            let (done, done_sound, failed, failed_sound) =
                (tr("notify.done"), tr("notify.done_sound"), tr("notify.failed"), tr("notify.failed_sound"));
            let ends = [
                ito_prefs::End { name: &done, color: l.c.done, sound: &done_sound },
                ito_prefs::End { name: &failed, color: l.c.err, sound: &failed_sound },
            ];
            // koyomi's own defaults: only 異常 is told, without a sound, and
            // the addresses are kept in the credential store.
            let defaults = crate::notify::parse(&koyomi_core::config::Config::default()).0.page(&crate::notify::EVENTS);
            if let Some(what) = ito_prefs::notify_page(ui, l, w, &ends, &mut n, &defaults, drafts) {
                test = Some((what, n.clone()));
            }
            if n != was {
                notify_new = Some(n);
            }
            let mut mails = notify.mail_places.clone();
            ito_prefs::section(ui, l, w.mail, |ui| {
                if let Some(k) = ito_prefs::mail_rows(ui, l, w, &mut mails, "koyomi secret set", drafts) {
                    mail_test = mails.get(k).cloned();
                }
            });
            if mails != notify.mail_places {
                mails_new = Some(mails);
            }
        }
        USERS => app.browser.users_page(ui, table),
        ACCOUNTS => app.browser.registry_page(ui, table, RegistryPage::Accounts),
        DATABASES => app.browser.registry_page(ui, table, RegistryPage::Databases),
        HOSTS => app.browser.registry_page(ui, table, RegistryPage::Hosts),
        BUCKETS => app.browser.registry_page(ui, table, RegistryPage::Buckets),
        MAILBOXES => app.browser.registry_page(ui, table, RegistryPage::Mailboxes),
        KEYS => {
            let layer_name = |n: &str| LAYERS.iter().find(|(id, _)| *id == n).map_or_else(|| n.to_owned(), |(_, k)| tr(k));
            let desc = |d: &str| table.tr_desc(d).to_owned();
            // Keys koyomi reads before the keymap: bound, they never run.
            let extra = |_: &str, on: &[ito_keys::Key]| -> Vec<String> {
                crate::keymap::Keys::fixed(on).map(|k| tr("keys.fixed").replace("{}", &k.human())).into_iter().collect()
            };
            let table = ito_prefs::KeyTable {
                now: &layers,
                built_in: crate::keymap::Keys::built_in_layers(),
                noop: crate::keymap::NOOP,
                own: &own,
                layer_name: &layer_name,
                tr: &desc,
                extra: &extra,
                unbound: &[],
                sup: false,
            };
            key_edits.extend(ito_prefs::keys_page(ui, l, kw, &table, keys_edit).unwrap_or_default());
        }
        ADVANCED if far => {
            far_line(ui, &far_text);
            advanced.extend(ito_prefs::advanced_card(ui, l, more, false));
        }
        ADVANCED => advanced.extend(ito_prefs::advanced_card(ui, l, more, state_dir.is_some())),
        ABOUT => {
            let system = ito_bugreport::os_line();
            let license = Some(env!("CARGO_PKG_LICENSE")).filter(|x| !x.is_empty());
            let about =
                ito_prefs::About { version: env!("CARGO_PKG_VERSION"), commit: option_env!("KOYOMI_COMMIT"), system: &system, license };
            bug |= !ito_prefs::about_page(ui, l, more, w, "koyomi", &about).is_empty();
        }
        _ => {}
    });
    if bug {
        app.report_bug(ui.ctx());
    }
    if open_config {
        app.browser.open_config_edit();
    }
    app.browser.config_box(ui.ctx(), &app.table);
    app.browser.token_box(ui.ctx(), &app.table);
    // The numbers typed: whole, 1 or more; else told off under the section.
    let mut limit_edits: Vec<(&'static str, String)> = Vec::new();
    for (key, text) in limit_typed {
        match text.trim().parse::<u64>() {
            Ok(n) if n >= 1 => limit_edits.push((key, n.to_string())),
            _ => app.prefs.config_warn = Some(tr("settings.bad_number")),
        }
    }
    if let Some(d) = keep_pick {
        let now = app.prefs.config.keep_days;
        if d < now {
            // Shorter: the box first, with what would go.
            let today = ito_common::now::now_local().date_naive();
            let lost = state_dir.as_deref().map_or(0, |s| koyomi_core::retain::days_lost(s, today, now, d));
            app.keep_ask = Some((d, lost));
        } else {
            limit_edits.push(("keep_days", d.to_string()));
        }
    }
    if let Some(on) = archive_pick {
        limit_edits.push(("archive", on.to_string()));
    }
    if let Some(name) = shell_pick {
        limit_edits.push(("shell", format!("\"{name}\"")));
    }
    if let Some(on) = watch_pick {
        app.prefs.set_watch_remotes(on, &no_folder);
    }
    app.prefs.limits_edit(limit_edits, &no_folder);
    if let Some((what, n)) = test {
        crate::notify::test(&n, what, &tr("notify.test_title"), &tr("notify.test_body"));
    }
    if let Some(n) = notify_new {
        app.prefs.notify_edit(n, &no_folder);
    }
    if let Some(m) = mail_test {
        app.prefs.mail_test(m, tr("notify.test_title"), tr("notify.test_body"));
    }
    if let Some(m) = mails_new {
        app.prefs.mails_edit(m, &no_folder);
    }
    if forget {
        if let Some(dir) = &state_dir {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _ = ito_prefs::forget_place(&dir);
            });
        }
    }
    if let Some(on) = startup_toggled {
        set_startup(ui.ctx(), app, on);
    }
    for a in advanced {
        match a {
            ito_prefs::Advanced::OpenConfigDir => match app.prefs.base.clone() {
                Some(b) => app.prefs.open_dir(b),
                None => app.scale_note = Some(no_folder.clone()),
            },
            ito_prefs::Advanced::OpenStateDir => {
                if let Some(d) = state_dir.clone() {
                    app.prefs.open_dir(d);
                }
            }
            // common.toml and keymap.toml are moved aside on a thread; the
            // definitions and koyomi.toml are the work, and stay (the
            // Notifications page has its own ↺, which writes `[notify]`).
            ito_prefs::Advanced::Reset => app.prefs.reset(&no_folder),
        }
    }
    if out.close {
        app.settings_open = false;
    }
    match out.open_file {
        Some(0) => app.prefs.open_file(ito_common::common_path, &no_folder),
        Some(_) => app.prefs.open_file(crate::prefs::config_path, &no_folder),
        None => {}
    }
    // Gone to another page: the keys stop being taken for this one.
    if app.prefs.state.page != KEYS {
        app.prefs.keys_edit.capture = None;
        app.prefs.keys_edit.listening = false;
    }
    app.prefs.keymap_edit(key_edits, &no_folder);
    app.prefs.common_change(asked, &no_folder);
}

/// koyomi's own settings file, opened from the foot of the settings' list.
const CONFIG_FILE: &str = "koyomi.toml";

/// The choices of the default shell: the shells this engine has (all five while it has not said), and the one
/// `koyomi.toml` wrote when it is not among them (said so). The index of the one now, and each choice's name and words.
/// Not written: the engine's default, which the snapshot carries.
fn shell_choices(engine: (&[crate::proto::Shell], &str), written: Option<&str>, t: &ito_i18n::Table) -> (usize, Vec<(String, String)>) {
    let (found, default) = engine;
    let mut names: Vec<String> = if found.is_empty() {
        koyomi_core::shell::SHELLS.iter().map(|s| (*s).to_owned()).collect()
    } else {
        found.iter().map(|s| s.name.clone()).collect()
    };
    let now = written.filter(|w| !w.is_empty()).unwrap_or(default);
    let mut words: Vec<(String, String)> = Vec::new();
    if !now.is_empty() && !names.iter().any(|n| n == now) {
        names.push(now.to_owned());
        words.push((now.to_owned(), t.trf("field.shell_missing", &[now])));
    }
    let mut out: Vec<(String, String)> =
        names.iter().filter(|n| !words.iter().any(|(w, _)| w == *n)).map(|n| (n.clone(), n.clone())).collect();
    out.extend(words);
    let at = out.iter().position(|(n, _)| n == now).unwrap_or(0);
    (at, out)
}

/// The choices for `keep_days`, as DECISIONS.md has them: no "never".
pub const KEEP_DAYS: [u32; 5] = [30, 60, 90, 180, 365];

/// The box that asks before `keep_days` is shortened, over the settings:
/// how many days' records go at once. Enter shortens, Esc keeps the days.
fn keep_ask(ui: &mut Ui, app: &mut App, no_folder: &str) {
    let Some((new, lost)) = app.keep_ask else { return };
    let t = &app.table;
    let now = app.prefs.config.keep_days;
    let title = t.tr("settings.keep_ask_title").to_owned();
    let rows = [(t.tr("settings.keep_days").to_owned(), t.trf("settings.keep_ask_change", &[&now.to_string(), &new.to_string()]))];
    // With archive on, the days are put into archive/, not gone for good.
    let body_key = if app.prefs.config.archive { "settings.keep_ask_body_archive" } else { "settings.keep_ask_body" };
    let body = [if lost == 0 { t.tr("settings.keep_ask_none").to_owned() } else { t.trf(body_key, &[&lost.to_string()]) }];
    let buttons = [t.tr("settings.keep_ask_yes").to_owned(), t.tr("confirm.no").to_owned()];
    let q = ito_confirm::Question {
        title: &title,
        rows: &rows,
        body: &body,
        buttons: &buttons,
        marks: &[Some(ito_prefs::Mark::Trash), Some(ito_prefs::Mark::Close)],
        focus: Some(0),
        primary: Some(0),
        ..Default::default()
    };
    let key = ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
            Some(0)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            Some(1)
        } else {
            None
        }
    });
    match ito_confirm::show(ui.ctx(), &q, &crate::ui::Browser::style()).or(key) {
        Some(0) => {
            app.keep_ask = None;
            app.prefs.limits_edit(vec![("keep_days", new.to_string())], no_folder);
        }
        Some(_) => app.keep_ask = None,
        None => {}
    }
}

/// Take the OS's answer about starting at sign-in, and ask when nothing is
/// known yet (once the General page is up). The registry and the autostart
/// folders are read on a thread.
fn poll_startup(ui: &Ui, app: &mut App) {
    if let Some((v, problem)) = app.startup_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        app.startup = Some(v);
        app.startup_problem = problem;
        app.startup_rx = None;
    }
    if app.startup.is_none() && app.startup_rx.is_none() && app.prefs.state.page == GENERAL {
        app.startup_rx = Some(startup_task(ui.ctx(), None));
    }
}

/// The switch was clicked: set it on a thread, then read what the OS has.
/// The last click's reason goes away now.
fn set_startup(ctx: &egui::Context, app: &mut App, on: bool) {
    app.startup = None;
    app.startup_problem = None;
    app.startup_rx = Some(startup_task(ctx, Some(on)));
}

/// Read (and first set, when `set` is given) the sign-in start on a thread,
/// with why a set did not take: an exe in Temp is refused (TODO 89), and
/// a registry or file write can fail.
fn startup_task(ctx: &egui::Context, set: Option<bool>) -> std::sync::mpsc::Receiver<(bool, Option<String>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let got = match set {
            Some(on) => ito_common::startup::apply("koyomi", &crate::cli::startup_args(), on, crate::cli::refuse_temp),
            None => (ito_common::startup::enabled("koyomi"), None),
        };
        let _ = tx.send(got);
        ctx.request_repaint();
    });
    rx
}

/// The Theme page's sample: a card with a run of each colour (ended ok,
/// ended abnormally, running) as the tree draws them.
fn preview(ui: &mut Ui, sample: &str, runs: &[String]) {
    let c = ito_theme::colors();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(420.0), 96.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 8.0, c.bg);
    p.rect_stroke(r, 8.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
    let inner = r.shrink(12.0);
    let mono = FontId::monospace(12.0);
    p.text(inner.left_top(), Align2::LEFT_TOP, sample, FontId::proportional(12.0), c.dim);
    let marks = [("●", c.done), ("✗", c.err), ("▶", c.run)];
    let mut x = inner.left();
    for ((m, color), name) in marks.into_iter().zip(runs) {
        let name = name.as_str();
        let g = p.layout_no_wrap(format!("{m} {name}"), mono.clone(), color);
        if x + g.size().x > inner.right() {
            break;
        }
        let w = g.size().x;
        p.galley(egui::pos2(x, inner.top() + 30.0), g, color);
        x += w + 14.0;
    }
    let g = p.layout_no_wrap("09:00 nightly/backup".to_owned(), mono, c.fg);
    p.galley(egui::pos2(inner.left(), inner.top() + 54.0), g, c.fg);
    p.circle_filled(egui::pos2(inner.right() - 8.0, inner.top() + 8.0), 4.0, c.done);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_prefs::Page;

    #[test]
    fn pages_follow_ito_default_order() {
        let ours = [GENERAL, THEME, NOTIFY, USERS, ACCOUNTS, DATABASES, HOSTS, BUCKETS, MAILBOXES, KEYS, ADVANCED, ABOUT];
        assert_eq!(ours, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert!(ours.windows(2).all(|w| w[0] < w[1]));
        let has = [Page::Theme, Page::Keys, Page::Advanced];
        assert_eq!(ito_prefs::default_order(&has), [Page::General, Page::Theme, Page::Keys, Page::Advanced, Page::About]);
    }

    /// The Keys page's words, and koyomi's own, are found by the search.
    #[test]
    fn the_search_finds_the_keys_page() {
        let (t, errors) = crate::i18n::table("en");
        assert!(errors.is_empty(), "{errors:?}");
        let idx = index(Words::of("en"), MoreWords::of("en"), "en", &t, false);
        assert!(idx.iter().any(|(p, t)| *p == KEYS && t == "keymap.toml"));
        assert!(idx.iter().any(|(p, t)| *p == KEYS && t == "Everywhere"));
        assert!(idx.iter().any(|(p, t)| *p == GENERAL && t == "[window]"));
    }

    /// The new Accounts and Databases pages are found by their words in both languages.
    #[test]
    fn the_search_finds_the_accounts_and_databases_pages() {
        for lang in ["en", "ja"] {
            let (t, errors) = crate::i18n::table(lang);
            assert!(errors.is_empty(), "{errors:?}");
            let idx = index(Words::of(lang), MoreWords::of(lang), lang, &t, false);
            let has = |p: usize, s: &str| idx.iter().any(|(q, x)| *q == p && x == s);
            assert!(has(ACCOUNTS, "[[account]]") && has(ACCOUNTS, t.tr("settings.accounts")), "{lang}");
            assert!(has(ACCOUNTS, t.tr("registry.add_account")) && has(ACCOUNTS, t.tr("registry.secret_field")), "{lang}");
            assert!(has(DATABASES, "[[database]]") && has(DATABASES, t.tr("settings.databases")), "{lang}");
            assert!(has(DATABASES, t.tr("registry.add_database")) && has(DATABASES, t.tr("registry.tls_field")), "{lang}");
        }
    }

    /// The Hosts, Buckets and Mailboxes pages are found by their words in both languages.
    #[test]
    fn the_search_finds_the_hosts_buckets_and_mailboxes_pages() {
        for lang in ["en", "ja"] {
            let (t, errors) = crate::i18n::table(lang);
            assert!(errors.is_empty(), "{errors:?}");
            let idx = index(Words::of(lang), MoreWords::of(lang), lang, &t, false);
            let has = |p: usize, s: &str| idx.iter().any(|(q, x)| *q == p && x == s);
            assert!(has(HOSTS, "[[host]]") && has(HOSTS, t.tr("settings.hosts")), "{lang}");
            assert!(has(HOSTS, t.tr("registry.add_host")) && has(HOSTS, t.tr("registry.host_key_field")), "{lang}");
            assert!(has(BUCKETS, "[[bucket]]") && has(BUCKETS, t.tr("settings.buckets")), "{lang}");
            assert!(has(BUCKETS, t.tr("registry.add_bucket")) && has(BUCKETS, t.tr("registry.region_field")), "{lang}");
            assert!(has(MAILBOXES, "[[mailbox]]") && has(MAILBOXES, t.tr("settings.mailboxes")), "{lang}");
            assert!(has(MAILBOXES, t.tr("registry.add_mailbox")) && has(MAILBOXES, t.tr("registry.mailbox_mail")), "{lang}");
        }
    }

    /// The default shell's choices are the shells the engine has; the one written stays among them when the engine
    /// lacks it; and before the engine has said, every shell is offered.
    #[test]
    fn the_default_shell_picks_among_the_shells_the_engine_has() {
        let (t, _) = crate::i18n::table("en");
        let sh = |n: &str| crate::proto::Shell { name: n.into(), path: format!("/bin/{n}") };
        let found = [sh("bash"), sh("sh")];
        let names = |v: &[(String, String)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        let (at, v) = shell_choices((&found, "bash"), None, &t);
        assert_eq!((at, names(&v)), (0, vec!["bash".to_owned(), "sh".to_owned()]));
        assert_eq!(shell_choices((&found, "bash"), Some("sh"), &t).0, 1);
        let (at, v) = shell_choices((&found, "bash"), Some("zsh"), &t);
        assert_eq!((at, v[2].0.as_str(), v[2].1.as_str()), (2, "zsh", "zsh (not on this engine)"));
        let (at, v) = shell_choices((&[], ""), None, &t);
        assert_eq!((at, v.len()), (0, 5));
    }

    /// Home keeps every row; another PC's engine loses the engine's (Records, koyomi.toml, the whole
    /// Notifications page) and keeps this window's own.
    #[test]
    fn the_search_on_another_pcs_engine_drops_the_engines_rows_only() {
        let (t, _) = crate::i18n::table("en");
        let (w, more) = (Words::of("en"), MoreWords::of("en"));
        let home = index(w, more, "en", &t, false);
        let far = index(w, more, "en", &t, true);
        let has = |idx: &[(usize, String)], p: usize, s: &str| idx.iter().any(|(q, x)| *q == p && x == s);
        for (p, s) in
            [(GENERAL, "keep_days"), (GENERAL, "log_max_mb"), (GENERAL, "koyomi.toml"), (NOTIFY, "[notify]"), (NOTIFY, "koyomi.toml")]
        {
            assert!(has(&home, p, s) && !has(&far, p, s), "{p} {s}");
        }
        for k in ["settings.records", "settings.keep_days", "settings.archive", "settings.concurrency", "settings.log_max"] {
            assert!(has(&home, GENERAL, t.tr(k)) && !has(&far, GENERAL, t.tr(k)), "{k}");
        }
        // On the Notifications page the only rows left are the window's own watch switch.
        let mine = [t.tr("settings.watch_section"), t.tr("settings.watch_remotes"), "watch_remotes"];
        let left: Vec<&str> = far.iter().filter(|(p, _)| *p == NOTIFY).map(|(_, x)| x.as_str()).collect();
        assert_eq!(left, mine, "{left:?}");
        for s in mine {
            assert!(has(&home, NOTIFY, s), "{s}");
        }
        assert!(home.iter().any(|(p, _)| *p == NOTIFY));
        // The window's own rows stay.
        for (p, s) in [(GENERAL, "language"), (GENERAL, "[window]"), (THEME, "theme"), (KEYS, "keymap.toml")] {
            assert!(has(&far, p, s), "{p} {s}");
        }
        // Advanced: on another PC's engine exactly the state folder's two rows are fewer.
        let adv = |idx: &[(usize, String)]| idx.iter().filter(|(p, _)| *p == ADVANCED).count();
        assert_eq!(adv(&far) + 2, adv(&home));
        for s in [more.state_dir, more.state_dir_note] {
            assert!(has(&home, ADVANCED, s) && !has(&far, ADVANCED, s), "{s}");
        }
        assert!(has(&far, ADVANCED, more.config_dir) && has(&far, ADVANCED, more.reset_label));
        // Only the engine's rows are gone.
        let gone = home.len() - far.len();
        assert_eq!(gone, home.iter().filter(|(p, x)| engine_row(*p, x, &t, more)).count());
    }
}
