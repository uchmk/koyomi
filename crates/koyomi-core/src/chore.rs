//! The everyday file chores a net can have as parts without a script:
//! delete old files, zip, copy with robocopy, empty a folder, send and fetch
//! files over SFTP or to and from a shared folder, call a web API. Each is
//! written in the file as its own `kind` and runs as a PowerShell script the
//! engine writes when the run starts, so it has the job's slot, log,
//! `timeout` and `retry`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::jobnet::{Jobnet, Judge, Part, Range};

/// One chore and what it works on. Paths may have `{date}` and `{param:…}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Chore {
    /// 古いファイルを消す: the files in `path` (not below it) matching `pattern`
    /// that were last written more than `keep_days` days ago.
    Delete {
        path: String,
        #[serde(default)]
        pattern: Option<String>,
        keep_days: u32,
    },
    /// ZIP にする: `from` (a file, a folder or a wildcard) into the `.zip` file `into`, replaced.
    Zip { from: String, into: String },
    /// robocopy で写す: the folder `from` and the folders below it into `into`; only
    /// the files matching `pattern` when it is there. Nothing is deleted in `into`.
    Copy {
        from: String,
        into: String,
        #[serde(default)]
        pattern: Option<String>,
    },
    /// フォルダーを空ける: everything in `path` goes; the folder stays.
    Empty { path: String },
    /// ファイルを送る: the files `from` (a folder or a wildcard here) into the folder
    /// `into`, either `sftp://[user@]host[:port]/folder` or a folder or share.
    Send {
        from: String,
        into: String,
        /// The SSH private key for `sftp://`; without it, the account's own keys.
        #[serde(default)]
        key: Option<String>,
    },
    /// ファイルを受け取る: the files `from`, either `sftp://[user@]host[:port]/folder/*.csv`
    /// or a folder or share (a wildcard allowed), into the folder `into` here.
    Fetch {
        from: String,
        into: String,
        #[serde(default)]
        key: Option<String>,
    },
    /// HTTP で呼ぶ: `method` to `url` with `body`, judged by the status code: one of
    /// `expect` (empty: 200–299) is 正常, another is 異常.
    Http {
        url: String,
        method: String,
        #[serde(default)]
        body: Option<String>,
        /// The body's type; without it, JSON (`application/json; charset=utf-8`).
        #[serde(default)]
        content_type: Option<String>,
        /// Header values may be `secret:<name>`; they reach the script as
        /// environment variables, so a secret is never written into it.
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        expect: Vec<u16>,
    },
}

/// The `kind` of each chore, in the order of the editor's shelf.
pub const KINDS: [&str; 7] = ["delete", "zip", "copy", "empty", "send", "fetch", "http"];

/// The methods `kind = "http"` takes.
pub const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// The longest `body` (characters): the script goes on the command line, which
/// Windows cuts at 32767 characters.
pub const BODY_MAX: usize = 8000;

/// The exit code of an `http` that got no answer (the name, the connection, TLS,
/// the time): 異常, told apart from a status code that is not expected (1).
const NO_ANSWER: i32 = 2;

/// The exit code of a `send` or `fetch` that found no file: 警告.
const NOTHING: i32 = 3;

impl Chore {
    /// The `kind` as written, and the key of its word.
    pub fn key(&self) -> &'static str {
        match self {
            Chore::Delete { .. } => "delete",
            Chore::Zip { .. } => "zip",
            Chore::Copy { .. } => "copy",
            Chore::Empty { .. } => "empty",
            Chore::Send { .. } => "send",
            Chore::Fetch { .. } => "fetch",
            Chore::Http { .. } => "http",
        }
    }

    /// A new, empty one of `kind` (for the shelf).
    pub fn blank(kind: &str) -> Option<Chore> {
        Some(match kind {
            "delete" => Chore::Delete { path: String::new(), pattern: None, keep_days: 30 },
            "zip" => Chore::Zip { from: String::new(), into: String::new() },
            "copy" => Chore::Copy { from: String::new(), into: String::new(), pattern: None },
            "empty" => Chore::Empty { path: String::new() },
            "send" => Chore::Send { from: String::new(), into: String::new(), key: None },
            "fetch" => Chore::Fetch { from: String::new(), into: String::new(), key: None },
            "http" => Chore::Http {
                url: String::new(),
                method: "GET".into(),
                body: None,
                content_type: None,
                headers: BTreeMap::new(),
                expect: Vec::new(),
            },
            _ => return None,
        })
    }

    /// Every path of it, to fill in `{date}` and `{param:…}`.
    pub fn paths_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![("path", path)],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![("from", from), ("into", into)],
            Chore::Send { from, into, key } | Chore::Fetch { from, into, key } => {
                let mut v = vec![("from", from), ("into", into)];
                if let Some(k) = key {
                    v.push(("key", k));
                }
                v
            }
            Chore::Http { url, body, .. } => {
                let mut v = vec![("url", url)];
                if let Some(b) = body {
                    v.push(("body", b));
                }
                v
            }
        }
    }

    /// Every path of it, as it is.
    pub fn paths(&self) -> Vec<&String> {
        match self {
            Chore::Delete { path, .. } | Chore::Empty { path } => vec![path],
            Chore::Zip { from, into } | Chore::Copy { from, into, .. } => vec![from, into],
            Chore::Send { from, into, key } | Chore::Fetch { from, into, key } => [from, into].into_iter().chain(key).collect(),
            Chore::Http { url, body, .. } => [url].into_iter().chain(body).collect(),
        }
    }

    /// Why it may not run as it stands: a path that is empty, a drive's root
    /// for `delete` and `empty`, a zip that does not end in `.zip`, an `sftp://`
    /// that cannot be read or is on the wrong side.
    pub fn check(&self) -> Result<(), String> {
        let need = |name: &str, p: &str| if p.trim().is_empty() { Err(format!("{name} が空")) } else { Ok(()) };
        match self {
            Chore::Delete { path, keep_days, .. } => {
                need("path", path)?;
                not_root(path)?;
                if *keep_days == 0 {
                    return Err("keep_days は 1 以上（全部を消すなら kind = \"empty\"）".into());
                }
            }
            Chore::Empty { path } => {
                need("path", path)?;
                not_root(path)?;
            }
            Chore::Zip { from, into } => {
                need("from", from)?;
                need("into", into)?;
                if !into.trim().to_ascii_lowercase().ends_with(".zip") {
                    return Err(format!("into は .zip のファイル: {into}"));
                }
            }
            Chore::Copy { from, into, .. } => {
                need("from", from)?;
                need("into", into)?;
            }
            Chore::Send { from, into, .. } => {
                need("from", from)?;
                need("into", into)?;
                if sftp(from).is_some() {
                    return Err("send の from はこのマシンのファイル（sftp:// は into に書く）".into());
                }
                sftp(into).transpose()?;
            }
            Chore::Fetch { from, into, .. } => {
                need("from", from)?;
                need("into", into)?;
                if sftp(into).is_some() {
                    return Err("fetch の into はこのマシンのフォルダー（sftp:// は from に書く）".into());
                }
                if let Some(at) = sftp(from).transpose()? {
                    if at.path == "." || at.path.ends_with('/') {
                        return Err(format!("fetch の from はファイルの名前か *.csv のような形まで書く: {from}"));
                    }
                }
            }
            Chore::Http { url, method, body, content_type, headers, expect } => {
                need("url", url)?;
                // A url that starts with `{param:…}` is told once it is filled in.
                let u = url.trim().to_ascii_lowercase();
                if !u.starts_with('{') && !u.starts_with("http://") && !u.starts_with("https://") {
                    return Err(format!("url は http:// か https:// で始める: {url}"));
                }
                if !METHODS.contains(&method.as_str()) {
                    return Err(format!("method は {} のどれか: {method}", METHODS.join("・")));
                }
                if body.is_some() && (method == "GET" || method == "HEAD") {
                    return Err(format!("method = \"{method}\" には body を書けない（送るなら method = \"POST\" など）"));
                }
                if body.as_ref().is_some_and(|b| b.chars().count() > BODY_MAX) {
                    return Err(format!("body は {BODY_MAX} 文字まで"));
                }
                if content_type.is_some() && body.is_none() {
                    return Err("content_type は body を書いたときだけ".into());
                }
                for k in headers.keys() {
                    header_name(k)?;
                }
                if let Some(c) = expect.iter().find(|c| !(100..=599).contains(*c)) {
                    return Err(format!("expect は 100〜599 の状態コード: {c}"));
                }
            }
        }
        Ok(())
    }

    /// The PowerShell script that does it, and how its exit code is judged.
    /// The paths are already filled in.
    pub fn script(&self) -> (String, Option<Judge>) {
        match self {
            Chore::Delete { path, pattern, keep_days } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $limit = (Get-Date).AddDays(-{keep_days})\n\
                     $old = @(Get-ChildItem -LiteralPath {p} -Filter {f} -File | Where-Object {{ $_.LastWriteTime -lt $limit }})\n\
                     foreach ($x in $old) {{ Remove-Item -LiteralPath $x.FullName -Force; Write-Output \"deleted: $($x.FullName)\" }}\n\
                     Write-Output \"$($old.Count) file(s) deleted\"\n",
                    p = quote(path),
                    f = quote(pattern.as_deref().unwrap_or("*")),
                );
                (s, None)
            }
            Chore::Zip { from, into } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $into = {i}\n\
                     $dir = Split-Path -Parent $into\n\
                     if ($dir) {{ New-Item -ItemType Directory -Force -Path $dir | Out-Null }}\n\
                     Compress-Archive -Path {f} -DestinationPath $into -Force\n\
                     Write-Output \"zipped: $into\"\n",
                    i = quote(into),
                    f = quote(from),
                );
                (s, None)
            }
            Chore::Copy { from, into, pattern } => {
                let only = pattern.as_deref().map(|p| format!(" {}", quote(p))).unwrap_or_default();
                // robocopy: 0–3 copied or nothing to do, 4–7 something did not match, 8 and up failed.
                let s = format!(
                    "& robocopy {f} {i}{only} /E /R:1 /W:1 /NP\n\
                     $c = $LASTEXITCODE\n\
                     if ($c -ge 8) {{ exit 2 }}\n\
                     if ($c -ge 4) {{ exit 1 }}\n\
                     exit 0\n",
                    f = quote(&folder(from)),
                    i = quote(&folder(into)),
                );
                (s, Some(Judge { warn: vec![Range { from: 1, to: 1 }], ..Default::default() }))
            }
            Chore::Empty { path } => {
                let s = format!(
                    "$ErrorActionPreference = 'Stop'\n\
                     $all = @(Get-ChildItem -LiteralPath {p} -Force)\n\
                     foreach ($x in $all) {{ Remove-Item -LiteralPath $x.FullName -Recurse -Force }}\n\
                     Write-Output \"$($all.Count) item(s) removed\"\n",
                    p = quote(path),
                );
                (s, None)
            }
            Chore::Send { from, into, key } => {
                let warn = Some(Judge { warn: vec![Range { from: NOTHING, to: NOTHING }], ..Default::default() });
                let s = match sftp(into) {
                    Some(Ok(to)) => format!(
                        "$ErrorActionPreference = 'Stop'\n\
                         $files = @(Get-ChildItem -Path {f} -File)\n\
                         if ($files.Count -eq 0) {{ Write-Output ('no file to send: ' + {f}); exit {NOTHING} }}\n\
                         $lines = @({cd}) + @($files | ForEach-Object {{ 'put \"' + $_.FullName.Replace('\\', '/') + '\"' }})\n\
                         {run}\
                         foreach ($x in $files) {{ Write-Output \"sent: $($x.FullName)\" }}\n",
                        f = quote(from),
                        cd = quote(&format!("cd \"{}\"", batch(&to.path))),
                        run = run_sftp(&to, key.as_deref(), "$lines", false),
                    ),
                    _ => shared(from, into, "send", "sent"),
                };
                (s, warn)
            }
            Chore::Fetch { from, into, key } => {
                let warn = Some(Judge { warn: vec![Range { from: NOTHING, to: NOTHING }], ..Default::default() });
                let s = match sftp(from) {
                    Some(Ok(at)) => {
                        // `ls -l` lists the files (a line starting with `-`) the path matches; each
                        // is then fetched by its own name from the folder of the path.
                        let dir = match at.path.rsplit_once('/') {
                            Some((d, _)) => format!("{}/", batch(d)),
                            None => String::new(),
                        };
                        format!(
                            "$ErrorActionPreference = 'Stop'\n\
                             $into = {i}\n\
                             New-Item -ItemType Directory -Force -Path $into | Out-Null\n\
                             $lines = @({ls})\n\
                             {list}\
                             $names = @($out | ForEach-Object {{ if ($_ -match '^-\\S+(?:\\s+\\S+){{7}}\\s+(.+)$') {{ ($Matches[1] -split '/')[-1] }} }})\n\
                             if ($names.Count -eq 0) {{ Write-Output ('no file to fetch: ' + {f}); exit {NOTHING} }}\n\
                             $local = (Resolve-Path -LiteralPath $into).ProviderPath.Replace('\\', '/').TrimEnd('/')\n\
                             $lines = @($names | ForEach-Object {{ 'get \"' + {dir} + $_.Replace('\\', '\\\\').Replace('\"', '\\\"') + '\" \"' + $local + '/' + $_ + '\"' }})\n\
                             {get}\
                             foreach ($n in $names) {{ Write-Output \"fetched: $n\" }}\n",
                            i = quote(into),
                            f = quote(from),
                            ls = quote(&format!("-ls -l \"{}\"", batch(&at.path))),
                            list = run_sftp(&at, key.as_deref(), "$lines", true),
                            dir = quote(&dir),
                            get = run_sftp(&at, key.as_deref(), "$lines", false),
                        )
                    }
                    _ => shared(from, into, "fetch", "fetched"),
                };
                (s, warn)
            }
            Chore::Http { url, method, body, content_type, headers, expect } => {
                (http(url, method, body, content_type, headers, expect), None)
            }
        }
    }

    /// The environment the script reads, beside the job's own: an `http` header's
    /// value (maybe `secret:<name>`) as `KOYOMI_HEADER_<n>`, in the order of `headers`.
    pub fn env(&self) -> Vec<(String, String)> {
        match self {
            Chore::Http { headers, .. } => headers.values().enumerate().map(|(i, v)| (format!("KOYOMI_HEADER_{i}"), v.clone())).collect(),
            _ => Vec::new(),
        }
    }
}

/// A header name as HTTP takes it; `Content-Type` is `content_type`.
pub fn header_name(k: &str) -> Result<(), String> {
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)) {
        return Err(format!("headers の名前に使えない: {k}（英数字と - など）"));
    }
    if k.eq_ignore_ascii_case("content-type") {
        return Err("Content-Type は headers でなく content_type に書く".into());
    }
    Ok(())
}

/// The script of an `http`: Windows PowerShell 5.1's `Invoke-WebRequest` (TLS 1.2
/// turned on, no progress bar: it slows the download and reaches stderr as CLIXML),
/// the status code and the start of the answer to the log, then the exit code: 0
/// when the code is expected, 1 when it is not, [`NO_ANSWER`] when there was no
/// answer.
fn http(
    url: &str,
    method: &str,
    body: &Option<String>,
    content_type: &Option<String>,
    headers: &BTreeMap<String, String>,
    expect: &[u16],
) -> String {
    let mut s = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $ProgressPreference = 'SilentlyContinue'\n\
         [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12\n\
         $p = @{{ Uri = {u}; Method = '{method}'; UseBasicParsing = $true; Headers = @{{}} }}\n",
        u = quote(url),
    );
    for (i, k) in headers.keys().enumerate() {
        s += &format!("$p.Headers[{}] = $env:KOYOMI_HEADER_{i}\n", quote(k));
    }
    if let Some(b) = body {
        // As UTF-8 bytes: a string would go as the system's code page.
        s += &format!(
            "$p.Body = [Text.Encoding]::UTF8.GetBytes({})\n$p.ContentType = {}\n",
            quote_raw(b),
            quote(content_type.as_deref().unwrap_or("application/json; charset=utf-8")),
        );
    }
    let ok = if expect.is_empty() {
        "$code -ge 200 -and $code -le 299".to_owned()
    } else {
        format!("@({}) -contains $code", expect.iter().map(u16::to_string).collect::<Vec<_>>().join(","))
    };
    s += &format!(
        "try {{ $r = Invoke-WebRequest @p; $code = [int]$r.StatusCode; $text = $r.Content }}\n\
         catch {{\n\
         \x20 $res = $_.Exception.Response\n\
         \x20 if ($null -eq $res) {{ Write-Output \"no answer: $($_.Exception.Message)\"; exit {NO_ANSWER} }}\n\
         \x20 $code = [int]$res.StatusCode\n\
         \x20 $text = $_.ErrorDetails.Message\n\
         }}\n\
         Write-Output \"HTTP $code\"\n\
         if ($text -is [byte[]]) {{ $text = [Text.Encoding]::UTF8.GetString($text) }}\n\
         if ($text) {{ if ($text.Length -gt 4000) {{ $text = $text.Substring(0, 4000) + ' ...' }}; Write-Output $text }}\n\
         if ({ok}) {{ exit 0 }}\n\
         exit 1\n",
    );
    s
}

/// Gives each chore of `net` (its paths filled in) the script that does it, so it
/// runs as a PowerShell job; a root or an empty path after the filling stops the run.
pub fn fill(net: &mut Jobnet) -> Result<(), String> {
    for j in &mut net.jobs {
        if let Part::Chore(c) = &j.part {
            c.check().map_err(|e| format!("{}: {e}", j.id))?;
            let (script, judge) = c.script();
            j.shell = Some("powershell".to_owned());
            j.script = Some(script);
            j.judge = judge;
            j.env.extend(c.env());
            j.run.clear();
        }
    }
    Ok(())
}

/// A PowerShell string that is `s` as it is: `'…'`, a `'` doubled.
fn quote(s: &str) -> String {
    quote_raw(s.trim())
}

/// [`quote`] without the trim, for a body. PowerShell also ends a `'…'` at the
/// typographic quotes, so they are doubled too.
fn quote_raw(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// A folder for robocopy: no `\` at the end (it would escape the closing quote),
/// but a drive's root keeps one as `D:\.`.
fn folder(p: &str) -> String {
    let t = p.trim().trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.ends_with(':') {
        format!("{t}\\.")
    } else {
        t.to_owned()
    }
}

/// `delete` and `empty` never work on a drive or share root.
fn not_root(path: &str) -> Result<(), String> {
    let t = path.trim().trim_end_matches(['\\', '/']);
    let share_root = t.strip_prefix("\\\\").is_some_and(|rest| rest.split(['\\', '/']).filter(|s| !s.is_empty()).count() <= 2);
    if t.is_empty() || (t.len() == 2 && t.ends_with(':')) || share_root {
        return Err(format!("ドライブや共有の一番上は扱わない: {path}"));
    }
    Ok(())
}

/// Where an `sftp://[user@]host[:port]/path` points.
#[derive(Debug, PartialEq, Eq)]
pub struct Sftp {
    /// `user@host` or `host`, as ssh takes it.
    pub dest: String,
    pub port: Option<u16>,
    /// The path on the server: `/in` from `…/in`, `in` (below the home) from `…/~/in`,
    /// `.` (the home) when there is none.
    pub path: String,
}

/// `s` read as an `sftp://` address; `None` when it is not one (a folder or share).
pub fn sftp(s: &str) -> Option<Result<Sftp, String>> {
    let t = s.trim();
    let rest = t.get(..7).filter(|p| p.eq_ignore_ascii_case("sftp://")).map(|_| &t[7..])?;
    Some(read_sftp(rest).map_err(|e| format!("{e}: {t}")))
}

fn read_sftp(rest: &str) -> Result<Sftp, String> {
    let (auth, path) = rest.split_once('/').unwrap_or((rest, ""));
    // A `:` inside `{param:…}` is not a port.
    let (dest, port) = match auth.rsplit_once(':') {
        Some((d, p)) if !p.contains('}') => match p.parse::<u16>() {
            Ok(n) if n > 0 => (d, Some(n)),
            _ => return Err(format!("ポートは 1〜65535 の数で書く（{p}）")),
        },
        _ => (auth, None),
    };
    let host = dest.rsplit_once('@').map_or(dest, |(_, h)| h);
    if host.is_empty() || dest.starts_with('@') {
        return Err("sftp:// には [ユーザー@]ホスト が要る".into());
    }
    // A leading `-` would be read by sftp.exe as an option.
    if dest.starts_with('-') || host.starts_with('-') || dest.chars().any(|c| c.is_whitespace() || "\"'`;&|<>".contains(c)) {
        return Err("ユーザーとホストに使えない文字がある".into());
    }
    if path.contains(['"', '\n', '\r']) {
        return Err("パスに \" や改行は使えない".into());
    }
    let path = match path.strip_prefix('~') {
        _ if path.is_empty() || path == "~" => ".".to_owned(),
        Some(below) => below.trim_start_matches('/').to_owned(),
        None => format!("/{path}"),
    };
    Ok(Sftp { dest: dest.to_owned(), port, path })
}

/// A path for a line of sftp's batch file, inside `"…"`.
fn batch(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The lines that write the batch file from `lines` (a PowerShell variable), run
/// Windows' own sftp.exe on it without asking anything (no password, no unknown
/// host), and stop with 2 when it fails. `capture` keeps its output in `$out`.
fn run_sftp(to: &Sftp, key: Option<&str>, lines: &str, capture: bool) -> String {
    let port = to.port.map(|p| format!(" -P {p}")).unwrap_or_default();
    let key = key.filter(|k| !k.trim().is_empty()).map(|k| format!(" -i {}", quote(k))).unwrap_or_default();
    let (keep, show) = if capture { ("$out = @(", ")") } else { ("", "") };
    format!(
        "$sftp = Join-Path $env:SystemRoot 'System32\\OpenSSH\\sftp.exe'\n\
         if (-not (Test-Path -LiteralPath $sftp)) {{ $sftp = 'sftp' }}\n\
         $b = [IO.Path]::GetTempFileName()\n\
         [IO.File]::WriteAllLines($b, [string[]]{lines})\n\
         $ErrorActionPreference = 'Continue'\n\
         {keep}& $sftp -b $b -o BatchMode=yes -o StrictHostKeyChecking=yes{port}{key} {dest}{show}\n\
         $c = $LASTEXITCODE\n\
         $ErrorActionPreference = 'Stop'\n\
         Remove-Item -LiteralPath $b -Force\n\
         if ($c -ne 0) {{ {print}exit 2 }}\n",
        dest = quote(&to.dest),
        print = if capture { "$out; " } else { "" },
    )
}

/// `send` and `fetch` between folders and shares: each file `from` matches is
/// copied into `into`, which is made when it is not there.
fn shared(from: &str, into: &str, verb: &str, done: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $files = @(Get-ChildItem -Path {f} -File)\n\
         if ($files.Count -eq 0) {{ Write-Output ('no file to {verb}: ' + {f}); exit {NOTHING} }}\n\
         $into = {i}\n\
         New-Item -ItemType Directory -Force -Path $into | Out-Null\n\
         foreach ($x in $files) {{ Copy-Item -LiteralPath $x.FullName -Destination $into -Force; Write-Output \"{done}: $($x.FullName)\" }}\n",
        f = quote(from),
        i = quote(into),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_quote_their_paths_and_copy_judges_robocopy() {
        let d = Chore::Delete { path: "D:\\logs\\it's".into(), pattern: Some("*.log".into()), keep_days: 30 };
        let (s, j) = d.script();
        assert!(s.contains("-LiteralPath 'D:\\logs\\it''s' -Filter '*.log'"), "{s}");
        assert!(s.contains("AddDays(-30)"), "{s}");
        assert!(j.is_none());
        let c = Chore::Copy { from: "D:\\out\\".into(), into: "E:\\".into(), pattern: None };
        let (s, j) = c.script();
        assert!(s.starts_with("& robocopy 'D:\\out' 'E:\\.' /E"), "{s}");
        assert_eq!(j.unwrap().warn, vec![Range { from: 1, to: 1 }]);
        let z = Chore::Zip { from: "D:\\out\\*.csv".into(), into: "D:\\arc\\a.zip".into() };
        assert!(z.script().0.contains("Compress-Archive -Path 'D:\\out\\*.csv'"));
    }

    #[test]
    fn roots_empty_paths_and_a_zip_without_zip_are_refused() {
        for p in ["C:\\", "C:", "\\\\nas\\share", "\\\\nas\\share\\", ""] {
            assert!(Chore::Empty { path: p.into() }.check().is_err(), "{p}");
        }
        assert!(Chore::Empty { path: "\\\\nas\\share\\tmp".into() }.check().is_ok());
        assert!(Chore::Delete { path: "D:\\logs".into(), pattern: None, keep_days: 0 }.check().is_err());
        assert!(Chore::Zip { from: "a".into(), into: "b.7z".into() }.check().is_err());
        assert!(Chore::Zip { from: "a".into(), into: "b.ZIP".into() }.check().is_ok());
    }

    #[test]
    fn an_sftp_address_reads_and_what_sftp_would_take_as_an_option_is_refused() {
        let a = sftp("sftp://partner@ftp.example.com:2222/in").unwrap().unwrap();
        assert_eq!(a, Sftp { dest: "partner@ftp.example.com".into(), port: Some(2222), path: "/in".into() });
        assert_eq!(sftp("SFTP://h/~/out/*.csv").unwrap().unwrap().path, "out/*.csv");
        assert_eq!(sftp("sftp://h").unwrap().unwrap().path, ".");
        assert_eq!(sftp("sftp://{param:user}@h/in").unwrap().unwrap().dest, "{param:user}@h");
        assert!(sftp("\\\\nas\\share").is_none());
        for bad in [
            "sftp:///in",
            "sftp://@h/in",
            "sftp://h:0/in",
            "sftp://h:x/in",
            "sftp://-oProxyCommand=x/in",
            "sftp://u@-h/in",
            "sftp://a b/in",
            "sftp://h/a\"b",
        ] {
            assert!(sftp(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn send_and_fetch_take_sftp_on_the_far_side_only() {
        let send = |from: &str, into: &str| Chore::Send { from: from.into(), into: into.into(), key: None }.check();
        let fetch = |from: &str, into: &str| Chore::Fetch { from: from.into(), into: into.into(), key: None }.check();
        assert!(send("D:\\out\\*.csv", "sftp://u@h/in").is_ok());
        assert!(send("D:\\out\\*.csv", "\\\\nas\\share\\in").is_ok());
        assert!(send("sftp://u@h/out", "D:\\in").is_err());
        assert!(send("D:\\out", "sftp://-x/in").is_err());
        assert!(fetch("sftp://u@h/out/*.csv", "D:\\in").is_ok());
        assert!(fetch("sftp://u@h/out/", "D:\\in").is_err());
        assert!(fetch("sftp://u@h", "D:\\in").is_err());
        assert!(fetch("D:\\out\\*.csv", "sftp://u@h/in").is_err());
        assert!(fetch("\\\\nas\\share\\out\\*.csv", "D:\\in").is_ok());
    }

    #[test]
    fn the_sftp_scripts_ask_nothing_and_judge_no_file_as_a_warning() {
        let s = Chore::Send { from: "D:\\out\\*.csv".into(), into: "sftp://u@h:2222/in box".into(), key: Some("C:\\keys\\id".into()) };
        let (script, judge) = s.script();
        assert!(script.contains("-o BatchMode=yes -o StrictHostKeyChecking=yes -P 2222 -i 'C:\\keys\\id' 'u@h'"), "{script}");
        assert!(script.contains("@('cd \"/in box\"')"), "{script}");
        assert!(script.contains("Get-ChildItem -Path 'D:\\out\\*.csv' -File"), "{script}");
        assert_eq!(judge.unwrap().warn, vec![Range { from: NOTHING, to: NOTHING }]);
        let f = Chore::Fetch { from: "sftp://u@h/~/out/*.csv".into(), into: "D:\\in".into(), key: None };
        let (script, _) = f.script();
        assert!(script.contains("@('-ls -l \"out/*.csv\"')"), "{script}");
        assert!(script.contains("$out = @(& $sftp -b $b -o BatchMode=yes -o StrictHostKeyChecking=yes 'u@h')"), "{script}");
        assert!(script.contains("'get \"' + 'out/' + "), "{script}");
        assert!(!script.contains(" -i "), "{script}");
        let share = Chore::Send { from: "D:\\out".into(), into: "\\\\nas\\share\\in".into(), key: None };
        let (script, judge) = share.script();
        assert!(script.contains("Copy-Item -LiteralPath $x.FullName -Destination $into -Force"), "{script}");
        assert!(!script.contains("sftp"), "{script}");
        assert!(judge.is_some());
    }

    fn http(method: &str, body: Option<&str>, headers: &[(&str, &str)], expect: &[u16]) -> Chore {
        Chore::Http {
            url: "https://api.example.com/jobs/{date}".into(),
            method: method.into(),
            body: body.map(Into::into),
            content_type: None,
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            expect: expect.to_vec(),
        }
    }

    #[test]
    fn an_http_script_reads_its_headers_from_the_environment_and_judges_by_the_code() {
        let h = http("POST", Some("{\"say\": \"it's\u{2019}\"}\n"), &[("X-Day", "1"), ("Authorization", "secret:api")], &[200, 202]);
        assert_eq!(h.check(), Ok(()));
        let (s, judge) = h.script();
        assert!(judge.is_none());
        assert!(s.contains("Uri = 'https://api.example.com/jobs/{date}'; Method = 'POST'"), "{s}");
        // In the order of the names, as env() gives the values.
        assert!(s.contains("$p.Headers['Authorization'] = $env:KOYOMI_HEADER_0\n$p.Headers['X-Day'] = $env:KOYOMI_HEADER_1"), "{s}");
        assert!(!s.contains("secret:api"), "{s}");
        assert_eq!(h.env(), vec![("KOYOMI_HEADER_0".to_owned(), "secret:api".to_owned()), ("KOYOMI_HEADER_1".to_owned(), "1".to_owned())]);
        // The body as it is, its quotes doubled, its last line kept.
        assert!(s.contains("UTF8.GetBytes('{\"say\": \"it''s\u{2019}\u{2019}\"}\n')"), "{s}");
        assert!(s.contains("$p.ContentType = 'application/json; charset=utf-8'"), "{s}");
        assert!(s.contains("if (@(200,202) -contains $code) { exit 0 }"), "{s}");
        assert!(s.contains("exit 2"), "{s}");
        let (s, _) = http("GET", None, &[], &[]).script();
        assert!(s.contains("if ($code -ge 200 -and $code -le 299) { exit 0 }"), "{s}");
        assert!(!s.contains("$p.Body"), "{s}");
        assert!(http("GET", None, &[], &[]).env().is_empty());
    }

    #[test]
    fn an_http_that_cannot_be_sent_as_written_is_refused() {
        let url = |u: &str| {
            let mut h = http("GET", None, &[], &[]);
            if let Chore::Http { url, .. } = &mut h {
                *url = u.into();
            }
            h.check()
        };
        assert!(url("").is_err());
        assert!(url("ftp://x").unwrap_err().contains("http:// か https://"));
        assert_eq!(url("HTTP://x"), Ok(()));
        assert_eq!(url("{param:api}/jobs"), Ok(()));
        assert!(http("get", None, &[], &[]).check().unwrap_err().contains("method は"));
        assert!(http("HEAD", Some("a"), &[], &[]).check().unwrap_err().contains("body を書けない"));
        assert!(http("POST", Some(&"a".repeat(BODY_MAX + 1)), &[], &[]).check().unwrap_err().contains("文字まで"));
        assert_eq!(http("POST", Some(&"あ".repeat(BODY_MAX)), &[], &[]).check(), Ok(()));
        assert!(http("GET", None, &[("content-type", "a")], &[]).check().unwrap_err().contains("content_type に書く"));
        assert!(http("GET", None, &[("X Day", "a")], &[]).check().unwrap_err().contains("名前に使えない"));
        assert!(http("GET", None, &[], &[600]).check().unwrap_err().contains("100〜599"));
        let mut typed = http("GET", None, &[], &[]);
        if let Chore::Http { content_type, .. } = &mut typed {
            *content_type = Some("text/plain".into());
        }
        assert!(typed.check().unwrap_err().contains("body を書いたときだけ"));
    }
}
