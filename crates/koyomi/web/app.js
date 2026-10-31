// koyomi's Web UI (TODO 81): reads the engine's HTTP API with the session cookie, starts, stops,
// reruns and skips runs, holds nets and shows a job's log. Data goes into the page only as text, never as HTML.
"use strict";

const STATES = {
  ok: "✓ 正常", warn: "△ 警告", error: "✕ 異常", timeout: "⏱ 異常（時間切れ）", killed: "■ 止めた",
  running: "▶ 実行中", running_error: "▶✕ 異常ありで実行中", waiting: "○ 実行待ち", held: "⏸ 保留",
  skipped: "⊘ スキップ", missed: "↷ 時刻を過ぎた", locked: "🔒 未確認で止めた", not_run: "· 未実行",
};
const STANDINGS = { confirmed: "確認済み", unconfirmed: "未確認", invalid: "読めない" };
const RANK = { view: 0, run: 1, edit: 2 };

let me = null;
let shown = null;

const $ = (id) => document.getElementById(id);

async function call(method, path, body) {
  const opts = { method, credentials: "same-origin", headers: {} };
  if (method !== "GET") opts.headers["X-Ito-Csrf"] = "1";
  if (body !== undefined) {
    opts.headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  const r = await fetch(path, opts);
  let json = null;
  try { json = await r.json(); } catch (_) { /* not JSON */ }
  if (r.status === 401) {
    signedOut();
    throw new Error("サインインが切れた");
  }
  if (!r.ok) throw new Error((json && json.error) || `${r.status}`);
  return json;
}

function say(text, bad) {
  const m = $("message");
  m.textContent = text || "";
  m.className = bad ? "bad" : "muted";
}

function cell(row, text, cls) {
  const td = document.createElement("td");
  td.textContent = text == null ? "" : String(text);
  if (cls) td.className = cls;
  row.appendChild(td);
  return td;
}

function button(td, label, onclick) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.addEventListener("click", onclick);
  td.appendChild(b);
}

const time = (t) => (t ? String(t).replace("T", " ").slice(0, 19) : "");
const can = (role) => me && RANK[me.role] >= RANK[role];

function signedOut() {
  me = null;
  $("main").hidden = true;
  $("signout").hidden = true;
  $("who").textContent = "";
  $("status").textContent = "";
  $("signin").hidden = false;
}

async function signIn(ev) {
  ev.preventDefault();
  const token = $("token").value;
  $("token").value = "";
  const r = await fetch("/session", {
    method: "POST", credentials: "same-origin", headers: { Authorization: `Bearer ${token}` },
  });
  if (!r.ok) {
    say("トークンが合わない", true);
    $("signin").hidden = false;
    return;
  }
  await start();
}

async function signOut() {
  try { await call("DELETE", "/session"); } catch (_) { /* gone already */ }
  signedOut();
}

async function start() {
  try {
    me = await call("GET", "/me");
  } catch (_) {
    signedOut();
    return;
  }
  $("signin").hidden = true;
  $("main").hidden = false;
  $("signout").hidden = false;
  $("who").textContent = `${me.who}（${me.role}）`;
  say("");
  await refresh();
}

async function refresh() {
  if (!me) return;
  try {
    const [health, nets, runs] = await Promise.all([call("GET", "/health"), call("GET", "/nets"), call("GET", "/runs")]);
    $("status").textContent = health.status;
    drawNets(nets);
    drawRuns(runs.runs);
    if (shown) await showRun(shown, false);
  } catch (e) {
    if (me) say(e.message, true);
  }
}

function drawNets(nets) {
  $("paused").hidden = !nets.paused;
  const body = $("nets");
  body.replaceChildren();
  for (const n of nets.nets) {
    const tr = document.createElement("tr");
    cell(tr, n.id);
    cell(tr, STANDINGS[n.standing] || n.standing, n.standing === "confirmed" ? "" : "warn");
    cell(tr, n.held ? "⏸" : "");
    const td = cell(tr, "");
    if (can("run") && n.standing === "confirmed") button(td, "流す", () => runNet(n.id, n.params || {}));
    if (can("run")) button(td, n.held ? "保留を解く" : "保留", () => holdNet(n.id, !n.held));
    body.appendChild(tr);
  }
}

function drawRuns(runs) {
  const body = $("runs");
  body.replaceChildren();
  for (const r of runs.slice().reverse()) {
    const tr = document.createElement("tr");
    cell(tr, time(r.started));
    cell(tr, r.title || r.jobnet);
    cell(tr, STATES[r.state] || r.state, `state-${r.state}`);
    cell(tr, time(r.ended));
    const td = cell(tr, "");
    button(td, "ジョブ", () => showRun(r.id, true));
    if (can("run") && !r.over) button(td, "止める", () => stopRun(r.id));
    if (can("run") && r.over) button(td, "再実行", () => rerun(r.id, null));
    body.appendChild(tr);
  }
}

// Each `[params]` value is asked for with its default filled in; only the changed ones are sent.
async function runNet(id, defaults) {
  const params = {};
  for (const [name, def] of Object.entries(defaults)) {
    const v = window.prompt(`${id} の ${name}（空のままなら既定）`, def);
    if (v === null) return;
    if (v !== "" && v !== def) params[name] = v;
  }
  const given = Object.entries(params).map(([k, v]) => `${k}=${v}`).join(" ");
  if (!window.confirm(given ? `${id} を今流す（${given}）` : `${id} を今流す`)) return;
  try {
    const body = Object.keys(params).length ? { net: id, params } : { net: id };
    const r = await call("POST", "/runs", body);
    say(`${id} を流した（${r.run}）`);
    await refresh();
  } catch (e) {
    say(e.message, true);
  }
}

async function stopRun(id) {
  if (!window.confirm(`${id} を止める（動いているジョブを止める）`)) return;
  try {
    await call("POST", `/runs/${encodeURIComponent(id)}/stop`);
    say(`${id} を止めた`);
    await refresh();
  } catch (e) {
    say(e.message, true);
  }
}

async function holdNet(id, on) {
  const what = on ? `${id} を保留にする（時刻が来ても流さない）` : `${id} の保留を解く`;
  if (!window.confirm(what)) return;
  try {
    await call("POST", on ? "/nets/hold" : "/nets/release", { net: id });
    say(on ? `${id} を保留にした` : `${id} の保留を解いた`);
    await refresh();
  } catch (e) {
    say(e.message, true);
  }
}

async function rerun(id, from) {
  const what = from ? `${id} を ${from} から再実行する` : `${id} を止まったところから再実行する（済んだジョブは引き継ぐ）`;
  if (!window.confirm(what)) return;
  try {
    const r = await call("POST", `/runs/${encodeURIComponent(id)}/rerun`, from ? { from } : {});
    say(`再実行した（${r.run}）`);
    shown = r.run;
    await refresh();
  } catch (e) {
    say(e.message, true);
  }
}

async function skipJob(run, job) {
  if (!window.confirm(`${job} をスキップする（⊘。後ろのジョブは進む）`)) return;
  try {
    await call("POST", `/runs/${encodeURIComponent(run)}/skip`, { job });
    say(`${job} をスキップした`);
    await refresh();
  } catch (e) {
    say(e.message, true);
  }
}

const NOT_STARTED = ["waiting", "held", "not_run"];

async function showRun(id, fresh) {
  try {
    const r = await call("GET", `/runs/${encodeURIComponent(id)}`);
    shown = id;
    $("detail").hidden = false;
    $("detail-title").textContent = `${r.title || r.jobnet}（${r.id}）`;
    const body = $("jobs");
    body.replaceChildren();
    for (const j of r.jobs) {
      const tr = document.createElement("tr");
      cell(tr, j.name || j.id);
      cell(tr, STATES[j.state] || j.state, `state-${j.state}`);
      cell(tr, time(j.started));
      cell(tr, time(j.ended));
      cell(tr, j.exit);
      const td = cell(tr, "");
      if (j.started) button(td, "ログ", () => showLog(r.id, j.id));
      if (can("run") && !r.over && NOT_STARTED.includes(j.state)) button(td, "スキップ", () => skipJob(r.id, j.id));
      if (can("run") && r.over) button(td, "ここから再実行", () => rerun(r.id, j.id));
      body.appendChild(tr);
    }
    if (fresh) {
      $("log").hidden = true;
      $("log-title").hidden = true;
    }
  } catch (e) {
    say(e.message, true);
  }
}

async function showLog(run, job) {
  try {
    const r = await call("GET", `/runs/${encodeURIComponent(run)}/log?job=${encodeURIComponent(job)}`);
    $("log-title").textContent = `ログ: ${job}`;
    $("log-title").hidden = false;
    $("log").textContent = r.log;
    $("log").hidden = false;
  } catch (e) {
    say(e.message, true);
  }
}

document.addEventListener("DOMContentLoaded", () => {
  $("signin-form").addEventListener("submit", signIn);
  $("signout").addEventListener("click", signOut);
  start();
  setInterval(refresh, 5000);
});
