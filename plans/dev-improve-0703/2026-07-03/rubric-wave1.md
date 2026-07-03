# Acceptance rubric — Wave 1 (hardening)

Verdict per item: APPROVE or REJECT: <file:line + concrete reproducible reason>. Vague critique = invalid verdict.

## Common (all items)
- [ ] Diff matches the brief; no out-of-scope files touched; no existing test modified/deleted.
- [ ] No new dependency; no signature change of frozen functions (probe_provider / add_provider_key / codex_mcp_add_args / onIdChange prop).
- [ ] Comments in English; style matches surrounding code; no AI attribution.
- [ ] Error paths: no swallowed errors, no data-loss on failure.
- [ ] i18n: any new user-visible backend string goes through tr/trv with ru/en/zh parity in i18n.rs.

## 1a (cmd injection)
- [ ] EVERY element that reaches `cmd /C codex` is checked (incl. server name, env k=v, command, args).
- [ ] Reject set exactly & | < > ^ % " ; rejection SURFACES as an error (not a silent skip).
- [ ] Unit tests cover command/args/env/name injection vectors; safe canonical config still passes.
- [ ] Adversarial: try to construct a bypass (e.g. metachar in env KEY not value; unicode homoglyph is out of scope; %VAR% expansion via %).

## 1b (probe https gate)
- [ ] valid_base_url runs first; https required; http allowed ONLY for localhost/127.*/::1.
- [ ] Returns the same {ok:false, detail} shape — UI contract intact.
- [ ] Adversarial: `http://localhost.evil.com`, `http://127.0.0.1.evil.com`, `https://169.254.169.254`, uppercase `HTTP://`, `http://[::1]:x` — verify each is handled correctly by the host parse.

## 1e (fork RAII)
- [ ] BOTH sites guarded (run_forks global flag AND run_fork_repo map entry); Drop always clears.
- [ ] Ordering semantics preserved: global flag set BEFORE emptiness check; per-repo check still sees FORKS_GLOBAL inside the lock.
- [ ] No double-remove / no removal of ANOTHER run's entry (guard owns its own path only).
- [ ] Reject-path behavior identical (err.fork_busy), pid still recorded for cancel.

## 1f (key rollback)
- [ ] Legacy `provider:{id}` deleted ONLY after successful JSON write; rollback restores exact pre-call keyring state (no orphan slots, keyCount consistent).
- [ ] Non-first-add path byte-identical in behavior.
- [ ] Unit test actually simulates write failure and asserts the legacy entry survives.

## 1c (dead grid)
- [ ] toggleBackground clears maximized only for the backgrounded pane's key.
- [ ] maxbar iterates activePanes; restoring a pane from background can't leave maximized pointing at a hidden pane.
- [ ] Trace: maximize A → background A → grid shows remaining active panes; maximize A → background B → A stays maximized.

## 1d (dead PTY)
- [ ] pty:exit → onIdChange(paneKey, null); pane excluded from broadcast/sendToAll/statusCounts.
- [ ] relaunch() after exit still works (trace the id-null path); attach-mirror panes unaffected negatively.
- [ ] closePane's peekMoved(sessionIds[key]) path still correct when id already nulled.
