// koyomi's Web UI (TODO 81), laid out as the window's 今日: the nets at the left, today's table (or a
// day of history) in the middle, and what is picked at the right. It reads the engine's HTTP API with
// the session cookie, starts, stops, reruns and skips runs, holds nets and shows a job's log.
// Data goes into the page only as text, never as HTML.
"use strict";

// The window's marks and words (lang/ja.toml [state]), in State::ALL's order.
const STATES = {
  ok: ["✓", "正常終了"],
  warn: ["△", "警告終了"],
  error: ["✕", "異常終了"],
  timeout: ["⏱", "打ち切り"],
  killed: ["■", "強制終了"],
  running: ["▶", "実行中"],
  running_error: ["▶✕", "実行中（異常あり）"],
  waiting: ["○", "待ち"],
  held: ["⏸", "保留"],
  skipped: ["⊘", "スキップ"],
  missed: ["↷", "時刻を過ぎた"],
  locked: ["🔒", "止めた"],
  not_run: ["·", "未実行"],
};
const ERRORS = ["error", "timeout", "killed", "locked"];
const NOT_STARTED = ["waiting", "held", "not_run"];
const STANDINGS = { confirmed: "承認済み", unconfirmed: "未承認", invalid: "誤り" };
const RANK = { view: 0, run: 1, edit: 2 };
const WEEKDAYS = "日月火水木金土";

const view = {
  me: null,
  nets: null,
  runs: [],
  health: null,
  down: false,
  tab: "today",
  day: null,
  filter: null,
  picked: null, // { run } or { net }
  run: null, // the picked run as /runs/<id> gave it
  log: null, // { run, job, text }
  menu: false,
  folded: new Set(),
};

const $ = (id) => document.getElementById(id);

function el(tag, attrs, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === "class") e.className = v;
    else if (k === "text") e.textContent = String(v);
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (k === "data") Object.assign(e.dataset, v);
    else e.setAttribute(k, v === true ? "" : String(v));
  }
  for (const kid of kids) if (kid != null) e.append(kid);
  return e;
}

function button(label, onclick, cls, attrs) {
  return el("button", { type: "button", class: cls, onclick, ...(attrs || {}) }, label);
}

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
    throw new Error("サインインが切れました");
  }
  if (!r.ok) throw new Error((json && json.error) || `${r.status}`);
  return json;
}

function say(text, bad) {
  const m = $("message");
  m.textContent = text || "";
  m.className = bad ? "message bad" : "message";
}

// The role on one net: what GET /nets says for a user with a role per net (TODO 101), else theirs.
const roleOn = (id) => {
  const n = netOf(id);
  return (n && n.role) || (view.me && view.me.role);
};
const canOn = (id, role) => view.me && RANK[roleOn(id)] >= RANK[role];

// Times as the window writes them. The engine sends local times without a zone.
const pad = (n) => String(n).padStart(2, "0");
const ymd = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const hms = (t) => (t ? String(t).slice(11, 19) : "");
const at = (t) => new Date(String(t).slice(0, 19));
function nextWords(t) {
  if (!t) return "";
  return String(t).slice(0, 10) === ymd(new Date()) ? hms(t) : `${String(t).slice(5, 10)} ${String(t).slice(11, 16)}`;
}
// `41分`, `1時間5分`, `12秒`: the two largest units (ito_common::text::duration_words).
function duration(ms) {
  const secs = Math.max(0, Math.floor(ms / 1000));
  const parts = [[Math.floor(secs / 86400), "日"], [Math.floor(secs / 3600) % 24, "時間"], [Math.floor(secs / 60) % 60, "分"], [secs % 60, "秒"]];
  const top = parts.findIndex((p) => p[0] > 0);
  if (top < 0) return "0秒";
  let out = `${parts[top][0]}${parts[top][1]}`;
  const next = parts[top + 1];
  if (next && next[0] > 0) out += `${next[0]}${next[1]}`;
  return out;
}
function dayWords(day) {
  const d = new Date(`${day}T00:00:00`);
  return `${day}(${WEEKDAYS[d.getDay()]})`;
}

// "mark word" in the state's colours.
function stateTag(state, extra) {
  const [mark, word] = STATES[state] || ["", state];
  return el("span", { class: `st s-${state}` }, el("span", { class: "m", text: mark }), word, extra || null);
}

function timeCell(text) {
  return text ? el("td", { class: "time", text }) : el("td", { class: "time none", text: "—" });
}

// What a dialog box asks; `fields` are [name, value] for text boxes. Resolves to the values, or null.
function ask(text, { yes = "はい", no = "やめる", danger = false, fields = [] } = {}) {
  const box = $("ask");
  $("ask-text").textContent = text;
  const holder = $("ask-fields");
  holder.replaceChildren();
  const inputs = fields.map(([name, value], i) => {
    const input = el("input", { id: `ask-f${i}`, value, autocomplete: "off" });
    holder.append(el("label", { for: `ask-f${i}`, text: name }), input);
    return [name, input];
  });
  $("ask-yes").textContent = yes;
  $("ask-yes").className = danger ? "danger" : "primary";
  $("ask-no").textContent = no;
  box.returnValue = "";
  return new Promise((resolve) => {
    box.addEventListener("close", () => {
      if (box.returnValue !== "yes") return resolve(null);
      resolve(Object.fromEntries(inputs.map(([name, input]) => [name, input.value])));
    }, { once: true });
    box.showModal();
    // A box that stops something starts on やめる, as the window's does.
    (inputs.length ? inputs[0][1] : $("ask-no")).focus();
  });
}

async function act(what, fn) {
  try {
    const said = await fn();
    say(said || what);
    await refresh();
  } catch (e) {
    say(e.message, true);
  }
}

// --- signing in -----------------------------------------------------------------------------------

function signedOut() {
  view.me = null;
  $("app").hidden = true;
  $("signout").hidden = true;
  $("engine").hidden = true;
  $("ha").hidden = true;
  $("who").textContent = "";
  $("foot").textContent = "";
  $("signin").hidden = false;
  $("token").focus();
}

async function signIn(ev) {
  ev.preventDefault();
  const token = $("token").value;
  $("token").value = "";
  const r = await fetch("/session", { method: "POST", credentials: "same-origin", headers: { Authorization: `Bearer ${token}` } });
  if (!r.ok) {
    $("signin-message").textContent = "トークンが合いません";
    return;
  }
  $("signin-message").textContent = "";
  await start();
}

async function signOut() {
  try { await call("DELETE", "/session"); } catch (_) { /* gone already */ }
  signedOut();
}

async function start() {
  try {
    view.me = await call("GET", "/me");
  } catch (_) {
    signedOut();
    return;
  }
  $("signin").hidden = true;
  $("app").hidden = false;
  $("signout").hidden = false;
  const per = view.me.roles ? Object.entries(view.me.roles).map(([p, r]) => `${p} は ${r}`) : [];
  $("who").textContent = `${view.me.who}（${[view.me.role, ...per].join("、")}）`;
  say("");
  await refresh();
}

// --- reading --------------------------------------------------------------------------------------

async function refresh() {
  if (!view.me) return;
  try {
    const runs = view.tab === "history" ? `/runs?from=${view.day}&to=${view.day}` : "/runs";
    const [health, nets, list] = await Promise.all([call("GET", "/health"), call("GET", "/nets"), call("GET", runs)]);
    view.health = health;
    view.nets = nets;
    view.runs = list.runs;
    view.down = false;
    if (view.picked && view.picked.run) {
      view.run = await call("GET", `/runs/${encodeURIComponent(view.picked.run)}`);
      if (view.log && view.log.run === view.run.id && !view.run.over) await readLog(view.log.run, view.log.job);
    }
  } catch (e) {
    if (!view.me) return;
    view.down = true;
    say(e.message, true);
  }
  draw();
}

async function readLog(run, job) {
  const r = await call("GET", `/runs/${encodeURIComponent(run)}/log?job=${encodeURIComponent(job)}`);
  view.log = { run, job, text: r.log };
}

const netOf = (id) => (view.nets ? view.nets.nets.find((n) => n.id === id) : null);

// The last run of each net in the runs at hand: what the tree marks it with.
function lastStates() {
  const last = new Map();
  for (const r of view.runs) {
    const had = last.get(r.jobnet);
    if (!had || had.started < r.started) last.set(r.jobnet, r);
  }
  return last;
}

// The rows of the table: today's starts still to come (待ち), then the runs, newest first.
function rows() {
  const out = [];
  const now = Date.now();
  if (view.tab === "today" && view.nets) {
    const coming = [];
    for (const n of view.nets.nets) {
      (n.today || []).forEach((t, i) => coming.push({ t, n, next: n.today[i + 1] || n.then }));
    }
    coming.sort((a, b) => (a.t < b.t ? -1 : a.t > b.t ? 1 : 0));
    for (const { t, n, next } of coming) {
      out.push({ key: `w:${n.id}:${t}`, state: "waiting", title: n.name, net: n.id, run: null, cells: [hms(t), "", "", "", nextWords(next)] });
    }
  }
  const runs = view.runs.slice().sort((a, b) => (a.started < b.started ? 1 : a.started > b.started ? -1 : 0));
  for (const r of runs) {
    const took = r.ended ? duration(at(r.ended) - at(r.started)) : `${duration(now - at(r.started))}…`;
    const n = netOf(r.jobnet);
    out.push({
      key: `r:${r.id}`,
      state: r.state,
      title: r.title || r.jobnet,
      net: r.jobnet,
      run: r.id,
      cells: [hms(r.planned), hms(r.started), hms(r.ended), took, view.tab === "today" && n ? nextWords(n.next) : ""],
    });
  }
  return out;
}

// --- drawing --------------------------------------------------------------------------------------

function draw() {
  if (!view.me) return;
  // Keyboard focus stays on what it was on: the page is drawn again every few seconds.
  const focused = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.key : null;
  drawBar();
  drawTree();
  drawCenter();
  drawPane();
  if (focused) {
    const again = document.querySelector(`[data-key="${CSS.escape(focused)}"]`);
    if (again) again.focus();
  }
}

function drawBar() {
  const chip = $("engine");
  chip.hidden = false;
  const paused = view.nets && view.nets.paused;
  chip.textContent = view.down ? "エンジンに繋がらない" : paused ? "スケジュール停止中" : "稼働中";
  chip.className = `chip ${view.down ? "down" : paused ? "paused" : "ok"}`;
  $("paused").hidden = !paused;
  drawHa();
  const parts = [];
  if (view.health && !view.down) parts.push(view.health.status);
  const running = view.runs.filter((r) => !r.over).length;
  if (running) parts.push(`実行中 ${running}`);
  if (view.nets) {
    const next = view.nets.nets.filter((n) => n.next).sort((a, b) => (a.next < b.next ? -1 : 1))[0];
    if (next) parts.push(`次の回 ${nextWords(next.next)} ${next.name}`);
  }
  $("foot").textContent = parts.join(" · ");
}

// With [standby] or [primary], which PC runs the schedules (GET /health's "ha", TODO 96).
function drawHa() {
  const chip = $("ha");
  const ha = !view.down && view.health ? view.health.ha : null;
  chip.hidden = !ha;
  if (!ha) return;
  // The same words and tones as the window's status bar (ui/ha.rs), with the time the engine
  // gives: when a standby took the schedules over, or last gave them back.
  let words;
  let tone;
  if (ha.role === "standby") {
    [words, tone] = ha.active
      ? [`${ha.watch} から引き継いで流している${ha.since ? `（${ha.since} から）` : ""}`, "run"]
      : [`待機中: ${ha.watch} を見張っている${ha.back ? `（${ha.back} に戻した）` : ""}`, "paused"];
  } else {
    [words, tone] = ha.hold === "held"
      ? [`${ha.standby} が引き継いでいる`, "paused"]
      : ha.hold === "asking" ? [`${ha.standby} に聞いている`, "paused"] : [`${ha.standby} が待機している`, "calm"];
  }
  chip.textContent = words;
  chip.className = `chip ${tone}`;
}

function drawTree() {
  const holder = $("groups");
  holder.replaceChildren();
  if (!view.nets) return;
  const todo = view.nets.nets.filter((n) => n.standing === "unconfirmed").length;
  $("todo").hidden = !todo;
  $("todo").textContent = `未承認 ${todo} 件`;
  const last = lastStates();
  const groups = new Map();
  for (const n of view.nets.nets) {
    if (!groups.has(n.group)) groups.set(n.group, []);
    groups.get(n.group).push(n);
  }
  for (const [group, nets] of groups) {
    const failed = nets.filter((n) => last.has(n.id) && ERRORS.includes(last.get(n.id).state)).length;
    const summary = el("summary", { data: { key: `g:${group}` } }, el("span", { text: group }), failed ? el("span", { class: "count", text: `✕ ${failed}` }) : null);
    const box = el("details", { open: !view.folded.has(group) }, summary);
    box.addEventListener("toggle", () => (box.open ? view.folded.delete(group) : view.folded.add(group)));
    for (const n of nets) {
      const r = last.get(n.id);
      const [mark] = r ? STATES[r.state] || [""] : [""];
      const on = view.picked && (view.picked.net === n.id || (view.run && view.picked.run && view.run.jobnet === n.id));
      const line = button(null, () => pick({ net: n.id }), `net${on ? " on" : ""}`, { data: { key: `n:${n.id}` }, "aria-current": on ? "true" : null });
      line.append(
        el("span", { class: `mark st s-${r ? r.state : "not_run"}` }, el("span", { class: "m", text: mark })),
        el("span", { class: "name", text: n.name }),
      );
      if (n.held) line.append(el("span", { class: "flag", title: "保留", text: "⏸" }));
      if (n.standing === "unconfirmed") line.append(el("span", { class: "flag", text: "未承認" }));
      if (n.standing === "invalid") line.append(el("span", { class: "problem", text: "誤り" }));
      box.append(line);
    }
    holder.append(box);
  }
}

function drawCenter() {
  const history = view.tab === "history";
  $("tab-today").setAttribute("aria-selected", String(!history));
  $("tab-history").setAttribute("aria-selected", String(history));
  $("day").hidden = !history;
  $("date").textContent = history ? "" : dayWords(ymd(new Date()));
  $("col-next").hidden = history;

  const all = rows();
  // The chips: every state a row is in, with how many; a click shows only its rows, a second all again.
  const counts = $("counts");
  counts.replaceChildren();
  for (const state of Object.keys(STATES)) {
    if (state === "not_run") continue;
    const n = all.filter((r) => r.state === state).length;
    const on = view.filter === state;
    if (!n && !on) continue;
    const [mark, word] = STATES[state];
    counts.append(button(`${mark} ${word} ${n}`, () => {
      view.filter = on ? null : state;
      draw();
    }, `tag s-${state}${on ? " on" : ""}`, { "aria-pressed": String(on), title: on ? "すべてを表示" : "これだけを表示", data: { key: `c:${state}` } }));
  }
  const shown = view.filter ? all.filter((r) => r.state === view.filter) : all;

  const body = $("rows");
  body.replaceChildren();
  for (const row of shown) {
    const on = view.picked && (row.run ? view.picked.run === row.run : !view.picked.run && view.picked.net === row.net);
    const tr = el("tr", { class: `s-${row.state}${on ? " on" : ""}`, tabindex: "0", "aria-selected": String(!!on), data: { key: row.key } });
    tr.append(el("td", {}, stateTag(row.state)), el("td", { class: "title", text: row.title }));
    row.cells.forEach((text, i) => {
      if (i === 4 && history) return;
      tr.append(timeCell(text));
    });
    const go = () => pick(row.run ? { run: row.run } : { net: row.net });
    tr.addEventListener("click", go);
    tr.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        go();
      }
    });
    body.append(tr);
  }
  const empty = $("empty");
  empty.hidden = shown.length > 0;
  empty.textContent = all.length ? "当てはまるものはありません" : history ? "この日の実行はありません" : "今日の実行はありません";
}

function pick(what) {
  view.picked = what;
  view.run = null;
  view.log = null;
  view.menu = false;
  refresh();
}

function drawPane() {
  const pane = $("pane");
  const picked = view.picked;
  const has = picked && (picked.run ? view.run : netOf(picked.net));
  pane.hidden = !has;
  $("app").classList.toggle("with-pane", !!has);
  if (!has) return;
  $("pane-actions").replaceChildren();
  $("pane-body").replaceChildren();
  $("pane-note").hidden = true;
  if (picked.run) drawRun(view.run);
  else drawNet(netOf(picked.net));
}

function note(text) {
  const n = $("pane-note");
  n.textContent = text;
  n.hidden = !text;
}

function drawRun(r) {
  $("pane-title").textContent = r.title || r.jobnet;
  $("pane-state").replaceChildren(stateTag(r.state));
  const span = r.started ? `${hms(r.started)}–${hms(r.ended)}` : "";
  const took = r.ended ? duration(at(r.ended) - at(r.started)) : "";
  $("pane-sub").textContent = [r.id, r.date, span, took].filter(Boolean).join(" · ");
  note(r.reason || "");

  const actions = $("pane-actions");
  if (canOn(r.jobnet, "run") && !r.over) {
    actions.append(button("強制終了", () => kill(r), "danger", { data: { key: "a:kill" } }));
  }
  if (canOn(r.jobnet, "run") && r.over) actions.append(rerunMenu(r));
  actions.append(button("ジョブネットを開く", () => pick({ net: r.jobnet }), "quiet", { data: { key: "a:net" } }));

  const body = $("pane-body");
  body.append(el("h3", { text: "ジョブ" }));
  const table = el("table", {}, el("thead", {}, el("tr", {}, ...["状態", "ジョブ", "開始", "終了", "終了コード", ""].map((h) => el("th", { text: h })))));
  const tbody = el("tbody");
  for (const j of r.jobs) {
    const name = el("td", { class: "name" }, j.name || j.id);
    const acts = el("td", { class: "acts" });
    if (j.started) acts.append(button("ログ", () => showLog(r.id, j.id), "small", { data: { key: `l:${j.id}` } }));
    if (canOn(r.jobnet, "run") && !r.over && NOT_STARTED.includes(j.state)) {
      acts.append(button("スキップ", () => skipJob(r, j), "small", { data: { key: `s:${j.id}` } }));
    }
    const on = view.log && view.log.job === j.id;
    const cls = [on ? "on" : "", j.reason ? "has-why" : ""].filter(Boolean).join(" ") || null;
    tbody.append(el("tr", { class: cls }, el("td", {}, stateTag(j.state)), name, timeCell(hms(j.started)), timeCell(hms(j.ended)), timeCell(j.exit == null ? "" : String(j.exit)), acts));
    // Why it waits or did not run, on a line of its own under the job, so the columns keep their width.
    if (j.reason) tbody.append(el("tr", { class: on ? "why on" : "why" }, el("td"), el("td", { class: "reason", colspan: "5", text: j.reason })));
  }
  table.append(tbody);
  body.append(el("div", { class: "table-wrap" }, table));
  if (view.log && view.log.run === r.id) {
    const job = r.jobs.find((j) => j.id === view.log.job);
    body.append(el("h3", { text: `ログ: ${job ? job.name || job.id : view.log.job}` }));
    body.append(el("pre", { class: "log", text: view.log.text || "ログはまだありません" }));
  }
}

// 再実行 ▾: from where it stopped, from a job, or past a failed job.
function rerunMenu(r) {
  const wrap = el("span", { class: "menu" });
  const open = button("再実行 ▾", () => {
    view.menu = !view.menu;
    draw();
  }, null, { "aria-haspopup": "true", "aria-expanded": String(view.menu), data: { key: "a:rerun" } });
  wrap.append(open);
  if (!view.menu) return wrap;
  const list = el("div", { class: "menu-list", role: "menu" });
  const item = (label, body, key) => list.append(button(label, () => {
    view.menu = false;
    rerun(r, label, body);
  }, null, { role: "menuitem", data: { key } }));
  item("止まったところから（済んだジョブは引き継ぐ）", {}, "m:rest");
  list.append(el("hr"));
  for (const j of r.jobs) item(`${j.name || j.id} から再実行`, { from: j.id }, `m:f:${j.id}`);
  const failed = r.jobs.filter((j) => ERRORS.includes(j.state));
  if (failed.length) list.append(el("hr"));
  for (const j of failed) {
    const name = j.name || j.id;
    item(`${name} の次から（${name} はスキップ）`, { next: j.id }, `m:n:${j.id}`);
    item(`${name} の次から（${name} は警告終了）`, { next: j.id, warn: true }, `m:w:${j.id}`);
  }
  wrap.append(list);
  return wrap;
}

function drawNet(n) {
  $("pane-title").textContent = n.name;
  const standing = el("span", { class: n.standing === "confirmed" ? "dim" : n.standing === "invalid" ? "st s-error" : "st s-warn", text: STANDINGS[n.standing] || n.standing });
  $("pane-state").replaceChildren(standing);
  $("pane-sub").textContent = n.id;
  if (n.held) note("ジョブネットを保留中です。時刻が来ても流しません。解くまで「今すぐ流す」もできません");
  else if (n.standing !== "confirmed") note("承認するまで流れません。承認は窓でします");

  const actions = $("pane-actions");
  if (canOn(n.id, "run")) {
    const ready = n.standing === "confirmed" && !n.held;
    actions.append(button("今すぐ流す", () => runNow(n), "primary", { disabled: !ready, data: { key: "a:now" } }));
    actions.append(n.held
      ? button("ジョブネットの保留を解く", () => hold(n, false), null, { data: { key: "a:hold" } })
      : button("ジョブネットを保留", () => hold(n, true), null, { data: { key: "a:hold" } }));
    if (n.next) actions.append(button("次の回を飛ばす", () => skipNext(n, false), null, { data: { key: "a:skip" } }));
    if ((n.skips || []).length) actions.append(button("飛ばしを戻す", () => skipNext(n, true), null, { data: { key: "a:unskip" } }));
  }

  const body = $("pane-body");
  body.append(el("h3", { text: "次の回" }), el("p", { class: "mono", text: n.next ? nextWords(n.next) : "—" }));
  if ((n.today || []).length) {
    body.append(el("h3", { text: "今日の予定" }));
    body.append(el("ul", { class: "list" }, ...n.today.map((t) => el("li", {}, stateTag("waiting", el("span", { class: "mono", text: ` ${hms(t)}` }))))));
  }
  if ((n.skips || []).length) {
    body.append(el("h3", { text: "飛ばす回" }));
    body.append(el("ul", { class: "list" }, ...n.skips.map((t) => el("li", {}, stateTag("skipped", el("span", { class: "mono", text: ` ${nextWords(t)}` }))))));
  }
  const runs = view.runs.filter((r) => r.jobnet === n.id).sort((a, b) => (a.started < b.started ? 1 : -1));
  body.append(el("h3", { text: view.tab === "history" ? "この日の回" : "今日の回" }));
  if (!runs.length) body.append(el("p", { class: "dim", text: "ありません" }));
  for (const r of runs) {
    const line = button(null, () => pick({ run: r.id }), "net", { data: { key: `pr:${r.id}` } });
    line.append(stateTag(r.state), el("span", { class: "name mono", text: ` ${hms(r.started)}–${hms(r.ended)}` }));
    body.append(line);
  }
}

// --- doing ----------------------------------------------------------------------------------------

async function kill(r) {
  const name = r.title || r.jobnet;
  if (!(await ask(`${name} を強制終了します。動いているジョブは止まり、まだのジョブは流れません`, { yes: "強制終了する", danger: true }))) return;
  await act(`${name} を強制終了しました`, () => call("POST", `/runs/${encodeURIComponent(r.id)}/stop`));
}

async function rerun(r, label, body) {
  if (!(await ask(`${r.title || r.jobnet}: ${label}`, { yes: "再実行" }))) return;
  await act("", async () => {
    const res = await call("POST", `/runs/${encodeURIComponent(r.id)}/rerun`, body);
    view.picked = { run: res.run };
    view.log = null;
    return `再実行しました（${res.run}）`;
  });
}

async function skipJob(r, j) {
  const name = j.name || j.id;
  if (!(await ask(`${name} をスキップしますか？（⊘ で終わり、後ろのジョブは進みます）`, { yes: `${name} をスキップ` }))) return;
  await act(`${name} をスキップしました`, () => call("POST", `/runs/${encodeURIComponent(r.id)}/skip`, { job: j.id }));
}

async function showLog(run, job) {
  try {
    await readLog(run, job);
    draw();
  } catch (e) {
    say(e.message, true);
  }
}

// Each `[params]` value is asked for with its default filled in; only the changed ones are sent.
async function runNow(n) {
  const defaults = Object.entries(n.params || {});
  const lead = defaults.length
    ? `${n.name} の [params] の値です。変えたものだけがこの実行に使われます（空のままにすると空の値）。`
    : `${n.name} を今すぐ流しますか？`;
  const given = await ask(lead, { yes: "今すぐ流す", fields: defaults });
  if (!given) return;
  const params = Object.fromEntries(Object.entries(given).filter(([k, v]) => v !== n.params[k]));
  await act("", async () => {
    const res = await call("POST", "/runs", Object.keys(params).length ? { net: n.id, params } : { net: n.id });
    view.picked = { run: res.run };
    view.tab = "today";
    return `${n.name} を流しました（${res.run}）`;
  });
}

async function hold(n, on) {
  const text = on ? `${n.name} を保留にしますか？時刻が来ても流れません` : `${n.name} の保留を解きますか？`;
  if (!(await ask(text, { yes: on ? "ジョブネットを保留" : "ジョブネットの保留を解く" }))) return;
  await act(on ? `${n.name} を保留にしました` : `${n.name} の保留を解きました`, () => call("POST", on ? "/nets/hold" : "/nets/release", { net: n.id }));
}

async function skipNext(n, cancel) {
  const text = cancel
    ? `${n.name} の飛ばす回を全部戻しますか？`
    : `${n.name} の次の回（${nextWords(n.next)}）を 1 回だけ流さずに、スキップとして残します。定義は変えません`;
  if (!(await ask(text, { yes: cancel ? "戻す" : "飛ばす" }))) return;
  await act("", async () => (await call("POST", "/nets/skip-next", { net: n.id, cancel })).said);
}

function showTab(tab) {
  view.tab = tab;
  view.filter = null;
  if (tab === "history" && !view.day) {
    view.day = ymd(new Date());
    $("day").value = view.day;
  }
  if (view.picked && view.picked.run) view.picked = null;
  refresh();
}

document.addEventListener("DOMContentLoaded", () => {
  $("signin-form").addEventListener("submit", signIn);
  $("signout").addEventListener("click", signOut);
  $("tab-today").addEventListener("click", () => showTab("today"));
  $("tab-history").addEventListener("click", () => showTab("history"));
  $("day").addEventListener("change", () => {
    if (!$("day").value) return;
    view.day = $("day").value;
    refresh();
  });
  $("pane-close").addEventListener("click", () => {
    view.picked = null;
    view.log = null;
    draw();
  });
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape" && view.menu) {
      view.menu = false;
      draw();
    }
  });
  start();
  setInterval(refresh, 5000);
});
