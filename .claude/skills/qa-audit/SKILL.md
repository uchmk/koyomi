---
name: qa-audit
description: Audit koyomi's test documentation (TESTING.md against the source, TESTING-CHECKS.md against TESTING.md). Use for checking the checklists; turning rows into tests is not available yet (no harness).
---

# QA audit（koyomi）

Two jobs in use. **Do one per run.** Your role and its limits are in [.claude/qa-role.md](../../qa-role.md). Read it first.

---

## Job 1 — TESTING.md against the code

TESTING.md has 120 `## N.` sections of `# | Do | Expect` rows. Work one section at a time. For each row:

1. **The keys it names.** Are they still bound to what the row says in the keymap (grep `crates/koyomi` for the key name)? A row naming a key that has moved will waste someone's afternoon.
2. **The words it expects on screen.** Grep `crates/koyomi` (`lang/ja.toml`, `lang/en.toml` and the Rust source) for them. A quoted string that is not in the source is stale.
3. **The version it is tagged with** (if any) should match when the feature landed, per CHANGELOG.md.
4. **Section numbers run 1..N with no gaps**, and each check id is `<section>.<n>`. No duplicate ids within a section.
5. **Can the row be run by someone holding only this file?** Does it name a key that exists under the reader's layout? Does it say what state the machine must be in first? When it says something happened, does it say how to see that it happened? A row that cannot be checked is not a check.
6. **Tests named in doc comments.** A test takes a row off the list by naming its id in a doc comment on the `#[test]` (`/// 1.1: …`). Check that each named id still exists in TESTING.md and that the test really does what the row says.

**Write the findings to your report** (`qa-reports/<date>-<topic>.md`, one row per finding):

```markdown
## TESTING.md — section 3（audited against <short sha>）

| # | 見つけたこと | 根拠 | どちらの間違いか |
| --- | --- | --- | --- |
| 3.4 | `koyomi startup oops` の終了コード | `cli/mod.rs:NNN` は 2 を返す | TESTING.md（1 と書いてある） |
```

What you may change in TESTING.md: the note under a section heading, only. Never a number, a check id or a row.

---

## Job 2 — TESTING-CHECKS.md against TESTING.md

```bash
cargo run -q -p koyomi --example make-testcheck -- --check
```

Exit 0 means the checklist is current. Exit 1 means it is out of date. **Report what `--check` says and which rows differ; do not regenerate the file.** Its ticks belong to the owner and the Windows session.

---

## Not in use yet: turning rows into tests

It needs a harness that runs the drawing code without a window. koyomi has none. If a section looks like it would need one, say so in the report as a proposal, and name the sections.

---

## Finishing a run

```bash
cargo fmt --all -- --check        # only your own files
cargo test -q -p koyomi
scripts/verify.sh                 # must print ALL OK
git diff                          # every added line inside a #[cfg(test)] module?
```

Then commit with only your own paths (`git add <path>`), and push to `main`. Do not bump the version or write CHANGELOG.md. Put the CHANGELOG line you would suggest in the report.

If verification does not pass and you cannot fix it inside a `#[cfg(test)]` module, `git restore` your own changes and write why in your report. A red commit is worse than none.
