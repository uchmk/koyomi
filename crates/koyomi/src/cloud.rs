//! `koyomi cloud run`: what a `send` into S3 or a `fetch` from it (`s3://bucket/key`)
//! runs, on Amazon S3 or an S3-compatible server (MinIO …) in `endpoint`. It reads
//! the job from the `KOYOMI_CLOUD_*` the engine sets and moves the files with
//! ito-cloud. The access key ID and the secret access key come already read from
//! the credential store, as any `secret:` of a job's environment does; by hand,
//! `secret:<name>` is read here.
//!
//! A `send` puts a file, a folder's files (not those below) or the files a `*.csv`
//! names under the key `into` names, as a folder; a `fetch` gets the keys `from`
//! names (`*` and `?` in the name only, not in the folders above it) into the
//! folder `into`. A big file goes in parts; a stopped or failed one leaves no part
//! behind (an upload is aborted, a download's `.part` deleted). There is no resume:
//! the job's `retry` starts over.
//!
//! It ends with 0 when every file moved, with [`NOTHING`] (a warning) when there
//! was none, and else with [`NO_EXIT`](crate::ssh) and why in the file
//! `KOYOMI_REASON` names. The files moved before the one that failed stay moved.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ito_cloud::{Error, Options, Target};
use koyomi_core::chore::{self, Chore, NOTHING, S3};

/// The most a run may take here: the job's own `timeout` ends it sooner.
const RUN_MAX: Duration = Duration::from_secs(30 * 24 * 3600);

/// The region when only `endpoint` is written (MinIO's own).
const DEFAULT_REGION: &str = "us-east-1";

/// The slowest download allowed, in bytes a second: a file gets ten minutes and a
/// second for each of these.
const SLOWEST: u64 = 100 * 1024;

/// A file this big or bigger logs each tenth of it as it goes.
const BIG: u64 = 64 * 1024 * 1024;

pub fn cmd(args: &[String]) -> i32 {
    match args {
        [op] if op == "run" => crate::ssh::run(exec),
        _ => {
            crate::say("koyomi: cloud takes run (what a send or fetch with s3:// does)", true);
            2
        }
    }
}

fn var(name: &str) -> Option<String> {
    crate::sftp::env_var("KOYOMI_CLOUD_", name)
}

/// Reads the job from the environment and runs it.
fn exec(stop: &AtomicBool) -> Result<i32, String> {
    let op = var("OP").ok_or("KOYOMI_CLOUD_OP がない（koyomi cloud run は s3:// の send か fetch のジョブが動かす）")?;
    let (from, into) = (var("FROM").unwrap_or_default(), var("INTO").unwrap_or_default());
    // Their values, read from the credential store when the job started; by hand,
    // still `secret:<name>`.
    let (key_id, password) = (var("KEY_ID"), var("PASSWORD"));
    // Checked as written: the values themselves are not `secret:`.
    let (id_mark, password_mark) =
        (key_id.as_ref().map(|_| "secret:key_id".to_owned()), password.as_ref().map(|_| "secret:password".to_owned()));
    let (region, endpoint) = (var("REGION"), var("ENDPOINT"));
    let job = match op.as_str() {
        "send" => Chore::Send { from, into, key: None, host_key: None, password: password_mark, key_id: id_mark, region, endpoint },
        "fetch" => Chore::Fetch { from, into, key: None, host_key: None, password: password_mark, key_id: id_mark, region, endpoint },
        _ => return Err(format!("KOYOMI_CLOUD_OP は send か fetch: {op}")),
    };
    go(&job, key_id, password, stop)
}

/// Runs `job` (a `send` or `fetch` with `s3://`, its `{…}` filled in) with the
/// access key ID `key_id` and the secret access key `password`: 0 when every file
/// moved, [`NOTHING`] when there was none, or why not.
fn go(job: &Chore, key_id: Option<String>, password: Option<String>, stop: &AtomicBool) -> Result<i32, String> {
    // Checked again now that each `{…}` is filled in.
    job.check()?;
    let (send, from, into, region, endpoint) = match job {
        Chore::Send { from, into, region, endpoint, .. } if job.cloud() => (true, from, into, region, endpoint),
        Chore::Fetch { from, into, region, endpoint, .. } if job.cloud() => (false, from, into, region, endpoint),
        _ => return Err("s3:// の send か fetch のジョブではない".into()),
    };
    let at = chore::s3(if send { into } else { from }).ok_or("s3:// の相手がない")??;
    let read = |v: Option<String>, name: &str| -> Result<String, String> {
        let v = crate::sftp::password(v.ok_or_else(|| format!("{name} がない"))?)?;
        Ok(v.trim().to_owned())
    };
    let endpoint = endpoint.as_deref().map(|e| e.trim().trim_end_matches('/').to_owned()).filter(|e| !e.is_empty());
    let target = Target::S3 {
        bucket: at.bucket.clone(),
        region: region.as_deref().map(str::trim).filter(|r| !r.is_empty()).unwrap_or(DEFAULT_REGION).to_owned(),
        // MinIO and most other S3-compatible servers take the bucket in the path.
        path_style: endpoint.is_some(),
        endpoint,
        key_id: read(key_id, "key_id")?,
        secret: read(password, "password")?,
        session_token: None,
    };
    if send {
        send_files(&target, &at, from.trim(), stop)
    } else {
        fetch_keys(&target, &at, into.trim(), stop)
    }
}

/// Puts the files `from` names under the folder `at`.
fn send_files(target: &Target, at: &S3, from: &str, stop: &AtomicBool) -> Result<i32, String> {
    let path = Path::new(from);
    let pattern = if path.is_dir() { path.join("*").display().to_string() } else { from.to_owned() };
    let files = crate::engine::files::find_files(&pattern);
    if files.is_empty() {
        crate::say(&format!("no file to send: {from}"), false);
        return Ok(NOTHING);
    }
    let folder = if at.key.is_empty() { String::new() } else { format!("{}/", at.key.trim_end_matches('/')) };
    let opts = Options { timeout: RUN_MAX, ..Options::default() };
    for (local, _, _) in &files {
        let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let key = format!("{folder}{name}");
        let bytes = ito_cloud::put(target, local, &key, &opts, stop, &mut progress(&name)).map_err(|e| why(&e, &at.bucket, &key))?;
        crate::say(&format!("sent: s3://{}/{key} ({bytes} bytes)", at.bucket), false);
    }
    Ok(0)
}

/// Gets the keys `at` names (in its folder, not below) into the folder `into`.
fn fetch_keys(target: &Target, at: &S3, into: &str, stop: &AtomicBool) -> Result<i32, String> {
    let opts = Options { timeout: RUN_MAX, ..Options::default() };
    let dir = &at.key[..at.key.rfind('/').map_or(0, |i| i + 1)];
    let found: Vec<_> = ito_cloud::list(target, &at.key, &opts, stop)
        .map_err(|e| why(&e, &at.bucket, &at.key))?
        .into_iter()
        // `*` matches `/` too: the keys below the folder are not this fetch's.
        .filter(|e| e.key.strip_prefix(dir).is_some_and(|n| !n.is_empty() && !n.contains('/')))
        .collect();
    if found.is_empty() {
        crate::say(&format!("no file to fetch: s3://{}/{}", at.bucket, at.key), false);
        return Ok(NOTHING);
    }
    // Before any is fetched: a name such as `..\x.csv` would land outside `into`.
    if let Some(bad) = found.iter().find(|e| !file_name(&e.key[dir.len()..])) {
        return Err(format!("このマシンのファイルの名前にできないキー: s3://{}/{}", at.bucket, bad.key));
    }
    let into = PathBuf::from(into);
    std::fs::create_dir_all(&into).map_err(|e| format!("into のフォルダーを作れない: {}: {e}", into.display()))?;
    for e in &found {
        let name = &e.key[dir.len()..];
        let o = Options { request_timeout: Duration::from_secs(600 + e.size / SLOWEST), ..opts };
        let bytes =
            ito_cloud::get(target, &e.key, &into.join(name), &o, stop, &mut progress(name)).map_err(|x| why(&x, &at.bucket, &e.key))?;
        crate::say(&format!("fetched: s3://{}/{} ({bytes} bytes)", at.bucket, e.key), false);
    }
    Ok(0)
}

/// Whether `name` can be a file's name in a folder here, as it is: not `.` or
/// `..`, and none of `\ / : * ? " < > |` or a control character.
fn file_name(name: &str) -> bool {
    !matches!(name, "" | "." | "..") && !name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
}

/// Logs each tenth of a file of [`BIG`] bytes or more as it moves.
fn progress(name: &str) -> impl FnMut(u64, u64) + '_ {
    let mut shown = 0;
    move |done, total| {
        if total < BIG {
            return;
        }
        let tenth = done.saturating_mul(10) / total;
        if tenth > shown && tenth < 10 {
            shown = tenth;
            crate::say(&format!("  {name}: {}%", tenth * 10), false);
        }
    }
}

/// Why a file did not move, in the words of the log.
fn why(e: &Error, bucket: &str, key: &str) -> String {
    let at = format!("s3://{bucket}/{key}");
    match e {
        Error::Auth(s) => format!("{at}: 断られた: {s}（key_id・password・region を確かめる）"),
        Error::NotFound => format!("{at}: バケットかキーがない"),
        Error::Cancelled => format!("{at}: 止めた（送りかけ・受けかけのものは残していない）"),
        Error::Timeout => format!("{at}: 時間切れ"),
        Error::Network(s) => format!("{at}: つなげない: {s}"),
        Error::Tls(s) => format!("{at}: TLS: {s}"),
        Error::Status { code, body } if body.is_empty() => format!("{at}: サーバーが {code} を返した"),
        Error::Status { code, body } => format!("{at}: サーバーが {code} を返した: {body}"),
        Error::Io(s) => format!("{at}: ファイル: {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_cloud::fake::{self, FakeS3};

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir = std::env::temp_dir().join(format!("koyomi-cloud-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn endpoint(s3: &FakeS3) -> String {
        let Target::S3 { endpoint, .. } = s3.target();
        endpoint.unwrap()
    }

    fn send(s3: &FakeS3, from: &Path, into: &str) -> Chore {
        Chore::Send {
            from: from.display().to_string(),
            into: into.into(),
            key: None,
            host_key: None,
            password: Some("secret:password".into()),
            key_id: Some("secret:key_id".into()),
            region: None,
            endpoint: Some(endpoint(s3)),
        }
    }

    fn fetch(s3: &FakeS3, from: &str, into: &Path) -> Chore {
        Chore::Fetch {
            from: from.into(),
            into: into.display().to_string(),
            key: None,
            host_key: None,
            password: Some("secret:password".into()),
            key_id: Some("secret:key_id".into()),
            region: None,
            endpoint: Some(format!("{}/", endpoint(s3))),
        }
    }

    fn run(job: &Chore, stop: bool) -> Result<i32, String> {
        go(job, Some(fake::KEY_ID.into()), Some(format!(" {} ", fake::SECRET)), &AtomicBool::new(stop))
    }

    #[test]
    fn a_send_puts_a_folders_files_a_patterns_or_one_under_the_key() {
        let s3 = FakeS3::start();
        let dir = Dir::new("send");
        let out = dir.0.join("out");
        std::fs::create_dir_all(out.join("sub")).unwrap();
        for (n, body) in [("a.csv", "1"), ("b.csv", "22"), ("c.txt", "333"), ("sub/d.csv", "4")] {
            std::fs::write(out.join(n), body).unwrap();
        }
        // A folder: its files, not those below.
        assert_eq!(run(&send(&s3, &out, "s3://bucket/daily"), false), Ok(0));
        assert_eq!(s3.keys(), ["daily/a.csv", "daily/b.csv", "daily/c.txt"]);
        assert_eq!(s3.object("daily/b.csv").as_deref(), Some(&b"22"[..]));
        // A pattern, into the bucket's top.
        assert_eq!(run(&send(&s3, &out.join("*.csv"), "s3://bucket"), false), Ok(0));
        assert!(s3.object("a.csv").is_some() && s3.object("b.csv").is_some() && s3.object("c.txt").is_none());
        // One file, into a folder written with its `/`.
        assert_eq!(run(&send(&s3, &out.join("c.txt"), "s3://bucket/one/"), false), Ok(0));
        assert_eq!(s3.object("one/c.txt").as_deref(), Some(&b"333"[..]));
        // Nothing to send is a warning, not a failure.
        assert_eq!(run(&send(&s3, &out.join("*.zip"), "s3://bucket/x/"), false), Ok(NOTHING));
        assert_eq!(run(&send(&s3, &dir.0.join("none.csv"), "s3://bucket/x/"), false), Ok(NOTHING));
        // Refused: says what to look at.
        s3.fail("PUT", 403, 1);
        let e = run(&send(&s3, &out.join("a.csv"), "s3://bucket/no/"), false).unwrap_err();
        assert!(e.starts_with("s3://bucket/no/a.csv: 断られた") && e.contains("key_id・password"), "{e}");
        // A stop before it starts.
        let e = run(&send(&s3, &out.join("a.csv"), "s3://bucket/no/"), true).unwrap_err();
        assert!(e.contains("止めた"), "{e}");
        assert!(s3.object("no/a.csv").is_none());
    }

    #[test]
    fn a_fetch_gets_the_matching_keys_of_its_folder_into_a_folder() {
        let s3 = FakeS3::start();
        let dir = Dir::new("fetch");
        for (k, body) in [("in/a.csv", "1"), ("in/b.csv", "22"), ("in/sub/c.csv", "3"), ("in/x.txt", "4"), ("other/d.csv", "5")] {
            s3.put_object(k, body.as_bytes());
        }
        let into = dir.0.join("got");
        assert_eq!(run(&fetch(&s3, "s3://bucket/in/*.csv", &into), false), Ok(0));
        let mut names: Vec<String> =
            std::fs::read_dir(&into).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["a.csv", "b.csv"]);
        assert_eq!(std::fs::read_to_string(into.join("b.csv")).unwrap(), "22");
        // One key.
        assert_eq!(run(&fetch(&s3, "s3://bucket/in/x.txt", &into), false), Ok(0));
        assert_eq!(std::fs::read_to_string(into.join("x.txt")).unwrap(), "4");
        // Nothing there is a warning.
        assert_eq!(run(&fetch(&s3, "s3://bucket/in/*.zip", &into), false), Ok(NOTHING));
        assert_eq!(run(&fetch(&s3, "s3://bucket/in/a.cs", &into), false), Ok(NOTHING));
        // A key that would land outside `into` fetches nothing.
        s3.put_object("bad/..\\up.csv", b"x");
        s3.put_object("bad/ok.csv", b"y");
        let far = dir.0.join("far");
        let e = run(&fetch(&s3, "s3://bucket/bad/*.csv", &far), false).unwrap_err();
        assert!(e.contains("ファイルの名前にできないキー") && e.contains("up.csv"), "{e}");
        assert!(!far.exists() && !dir.0.join("up.csv").exists());
        // A failed download leaves no `.part`.
        s3.fail("GET /bucket/in/a.csv", 500, 10);
        let e = run(&fetch(&s3, "s3://bucket/in/a.csv", &dir.0.join("again")), false).unwrap_err();
        assert!(e.starts_with("s3://bucket/in/a.csv: サーバーが 500 を返した"), "{e}");
        assert!(!dir.0.join("again").join("a.csv.part").exists() && !dir.0.join("again").join("a.csv").exists());
        // A stop.
        assert!(run(&fetch(&s3, "s3://bucket/in/*.csv", &into), true).unwrap_err().contains("止めた"));
    }

    #[test]
    fn a_job_is_checked_again_as_filled_in() {
        let s3 = FakeS3::start();
        let dir = Dir::new("check");
        let mut job = send(&s3, &dir.0, "s3://bucket/x/");
        if let Chore::Send { region, endpoint, .. } = &mut job {
            *region = Some("Tokyo".into());
            *endpoint = None;
        }
        assert!(run(&job, false).unwrap_err().contains("region は ap-northeast-1 のように"));
        let none = go(&send(&s3, &dir.0, "s3://bucket/x/"), None, None, &AtomicBool::new(false));
        assert_eq!(none, Err("key_id がない".into()));
        let copy = Chore::Copy { from: "a".into(), into: "b".into(), pattern: None };
        assert!(run(&copy, false).unwrap_err().contains("s3:// の send か fetch"));
    }

    #[test]
    fn small_names_and_progress() {
        assert!(file_name("a.csv") && file_name("売上 2026.csv"));
        for bad in ["", ".", "..", "a\\b", "a:b", "a?b", "a\nb"] {
            assert!(!file_name(bad), "{bad:?}");
        }
        assert_eq!(why(&Error::NotFound, "logs", "in/a.csv"), "s3://logs/in/a.csv: バケットかキーがない");
        // Small files say nothing; a big one, each tenth once.
        let mut p = progress("a.zip");
        p(10, 20);
        p(BIG / 2, BIG);
        p(BIG / 2 + 1, BIG);
        p(BIG, BIG);
    }
}
