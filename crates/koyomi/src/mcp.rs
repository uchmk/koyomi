//! `koyomi mcp`: Claude Code reads the runs and, once the person has agreed, starts and stops them.
//!
//! ```text
//! Claude Code ──stdio (JSON-RPC)── koyomi mcp ──the engine's door── koyomi engine
//! ```
//!
//! Every tool is a request the CLI makes too, so the engine decides what is allowed (a net that
//! is not approved does not run). The reading tools change nothing. `koyomi_run` and `koyomi_kill`
//! are marked as writing, so the client asks the person, and they also do nothing until called
//! again with `confirm = true`: without it they only say what they would do.

use std::io;

use ito_mcp::serde_json::{json, Value};
use ito_mcp::{arg_str, Answer, Server, Tool};

use crate::client;
use crate::proto::Request;

fn opt<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn confirmed(args: &Value) -> bool {
    args.get("confirm").and_then(Value::as_bool).unwrap_or(false)
}

/// The engine's answer, or what stopped it, for the model.
fn ask(req: &Request) -> Answer {
    client::call(req)
}

fn read_tools() -> Vec<Tool> {
    vec![
        Tool::new("koyomi_status", "Today's runs and the ones still going (TOML): state, net, start and end, each job.", |_| {
            ask(&Request::Runs)
        }),
        Tool::new("koyomi_list", "Every net definition and the warnings (TOML).", |_| ask(&Request::List)),
        Tool::new("koyomi_history", "The runs that started from one day to another (TOML), oldest first.", |a| {
            ask(&Request::History { from: arg_str(a, "from")?.to_owned(), to: arg_str(a, "to")?.to_owned() })
        })
        .input(json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "YYYY-MM-DD" },
                "to": { "type": "string", "description": "YYYY-MM-DD (at most a year after from)" },
            },
            "required": ["from", "to"],
        })),
        Tool::new("koyomi_log", "The tail of one job's log in a run.", |a| {
            ask(&Request::Log { run: arg_str(a, "run")?.to_owned(), job: arg_str(a, "job")?.to_owned() })
        })
        .input(json!({
            "type": "object",
            "properties": { "run": { "type": "string" }, "job": { "type": "string" } },
            "required": ["run", "job"],
        })),
    ]
}

fn write_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "koyomi_run",
            "Start a net now. Without confirm = true it only prints each job's command as it would run; ask the person, then call again with confirm = true.",
            |a| {
                let id = arg_str(a, "id")?.to_owned();
                let (date, from) = (opt(a, "date").map(str::to_owned), opt(a, "from").map(str::to_owned));
                if !confirmed(a) {
                    let preview = ask(&Request::Run { id, date, from, dry: true })?;
                    return Ok(format!("Nothing started. It would run:\n{preview}\nAsk the person, then call again with confirm = true."));
                }
                ask(&Request::Run { id, date, from, dry: false }).map(|run| format!("started: {run}"))
            },
        )
        .input(json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "group/name" },
                "date": { "type": "string", "description": "the business day, YYYY-MM-DD (today when left out)" },
                "from": { "type": "string", "description": "start at this job; the ones before it are skipped" },
                "confirm": { "type": "boolean", "description": "true only after the person agreed" },
            },
            "required": ["id"],
        }))
        .writes(),
        Tool::new("koyomi_kill", "Stop a run, or one job of it. Does nothing unless confirm = true; ask the person first.", |a| {
            let (run, job) = (arg_str(a, "run")?.to_owned(), opt(a, "job").map(str::to_owned));
            if !confirmed(a) {
                let what = job.as_deref().map_or_else(|| format!("run {run}"), |j| format!("job {j} of run {run}"));
                return Ok(format!("Nothing stopped. It would stop {what}. Ask the person, then call again with confirm = true."));
            }
            ask(&Request::Kill { run, job })
        })
        .input(json!({
            "type": "object",
            "properties": {
                "run": { "type": "string" },
                "job": { "type": "string", "description": "only this job (the whole run when left out)" },
                "confirm": { "type": "boolean", "description": "true only after the person agreed" },
            },
            "required": ["run"],
        }))
        .writes(),
    ]
}

fn server() -> Server {
    let mut s = Server::new("koyomi", env!("CARGO_PKG_VERSION")).instructions(
        "koyomi is a job scheduler. Read runs, history and logs freely. Starting or stopping a run needs the person's agreement: \
         call koyomi_run / koyomi_kill without confirm to see what would happen, ask, then call again with confirm = true.",
    );
    for t in read_tools().into_iter().chain(write_tools()) {
        s = s.tool(t);
    }
    s
}

/// `koyomi mcp`: serve on stdin/stdout until the client closes it.
pub fn run() -> i32 {
    match server().serve(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("koyomi mcp: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(req: Value) -> Value {
        let line = server().handle(&req.to_string()).unwrap();
        ito_mcp::serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn only_run_and_kill_write() {
        let v = reply(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
        let tools = v["result"]["tools"].as_array().unwrap();
        let mut writing: Vec<&str> =
            tools.iter().filter(|t| t["annotations"]["readOnlyHint"] != json!(true)).map(|t| t["name"].as_str().unwrap()).collect();
        writing.sort_unstable();
        assert_eq!(writing, ["koyomi_kill", "koyomi_run"]);
        assert_eq!(tools.len(), 6);
    }

    #[test]
    fn kill_without_confirm_stops_nothing() {
        let v = reply(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "koyomi_kill", "arguments": {"run": "r1", "job": "a"}}
        }));
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Nothing stopped"), "{text}");
        assert_ne!(v["result"]["isError"], json!(true));
    }
}
