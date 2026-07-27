#!/usr/bin/env node
// Scripted UI smoke test — the gate every other gate is blind to.
//
// cargo test / vitest / svelte-check all pass while a tab fails to render at runtime. This walks
// every sidebar tab of a RUNNING isolated instance over raw CDP and fails when a tab logs an error
// or renders nothing.
//
// It attaches only — it never starts or stops the app. Bring one up first:
//     pwsh -File tools/iso-test.ps1 -World      (+ -Build after Rust changes)
//     node tools/smoke.mjs
//     pwsh -File tools/iso-test.ps1 -Stop
//
// vite is pinned to 1420 with strictPort and the debug exe loads devUrl=1420, so the isolated
// instance and a normal `npm run tauri dev` CANNOT run at the same time — stop your own dev first.
//
// No dependencies: Node 24 ships global fetch + WebSocket, which is all CDP needs.

const CDP_PORT = Number(process.env.CASTELLYN_CDP_PORT ?? 9223);
// Sidebar buttons carry data-tab="<id>" purely so this walk has a locale-independent anchor —
// the visible label is translated, the id is not. Keep the attribute if you touch Sidebar.svelte.
const NAV = 'nav.sidebar button.nav-item';
const MIN_TEXT = 20; // chars of rendered text below which a tab counts as blank
const SETTLE_MS = 8000;
// How long to wait for the Svelte app to mount after attaching. Generous: on a cold vite start the
// dev server is still compiling when CDP already answers.
const MOUNT_MS = 60000;

if (process.argv.includes('--help') || process.argv.includes('-h')) {
  console.log(
    [
      'Usage: node tools/smoke.mjs',
      '',
      'Walks every sidebar tab of an already-running isolated Castellyn and reports',
      'console errors / blank tabs. Exit 0 = pass, 1 = a tab failed, 2 = nothing to attach to.',
      '',
      `Start the instance first:  pwsh -File tools/iso-test.ps1 -World   (CDP :${CDP_PORT})`,
      'Tear it down after:        pwsh -File tools/iso-test.ps1 -Stop',
      '',
      'vite is pinned to :1420 (strictPort), so the isolated instance and your own',
      '`npm run tauri dev` cannot run at the same time.',
      '',
      'Env: CASTELLYN_CDP_PORT overrides the port (matches iso-test.ps1 -CdpPort).'
    ].join('\n')
  );
  process.exit(0);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// A CDP RemoteObject -> something printable.
const describe = (a) =>
  a.value !== undefined ? String(a.value) : (a.description ?? a.unserializableValue ?? a.type);

function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    const pending = new Map();
    const listeners = [];
    let seq = 0;
    ws.onerror = () => reject(new Error(`CDP websocket failed: ${wsUrl}`));
    ws.onclose = () => {
      for (const p of pending.values()) p.reject(new Error('CDP socket closed mid-command'));
      pending.clear();
    };
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id === undefined) {
        for (const fn of listeners) fn(msg);
        return;
      }
      const p = pending.get(msg.id);
      if (!p) return;
      pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`${msg.error.message} (${msg.error.code})`));
      else p.resolve(msg.result);
    };
    ws.onopen = () =>
      resolve({
        send: (method, params) =>
          new Promise((res, rej) => {
            const id = ++seq;
            pending.set(id, { resolve: res, reject: rej });
            ws.send(JSON.stringify({ id, method, params }));
          }),
        on: (fn) => listeners.push(fn),
        close: () => ws.close()
      });
  });
}

async function evaluate(cdp, expression) {
  const r = await cdp.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true
  });
  if (r.exceptionDetails) {
    throw new Error(
      `evaluate failed: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`
    );
  }
  return r.result.value;
}

// Click a tab, then wait until it is the active one AND its main region has stopped changing.
// Two identical consecutive reads is the settle signal — tabs fetch lazily, so a single read
// right after the click would sample the empty pre-load state.
async function openTab(cdp, id) {
  const sel = `${NAV}[data-tab=${JSON.stringify(id)}]`;
  await evaluate(cdp, `document.querySelector(${JSON.stringify(sel)}).click()`);
  const probe = `(() => {
    const btn = document.querySelector(${JSON.stringify(sel)});
    const main = document.querySelector('main');
    return {
      active: btn?.getAttribute('aria-current') === 'page',
      text: (main?.innerText ?? '').trim()
    };
  })()`;
  const deadline = Date.now() + SETTLE_MS;
  let prev = null;
  for (;;) {
    const s = await evaluate(cdp, probe);
    if (s.active && s.text.length >= MIN_TEXT && s.text === prev?.text) return s;
    if (Date.now() > deadline) return s;
    prev = s;
    await sleep(250);
  }
}

async function main() {
  let targets;
  try {
    targets = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
  } catch (e) {
    console.error(`✗ Nothing answering CDP on 127.0.0.1:${CDP_PORT} (${e.message}).`);
    console.error('  Start the isolated instance first:  pwsh -File tools/iso-test.ps1 -World');
    process.exit(2);
  }
  const target = targets.find((t) => t.type === 'page' && t.webSocketDebuggerUrl);
  if (!target) {
    console.error(`✗ CDP is up on :${CDP_PORT} but exposes no page target.`);
    console.error(`  Targets: ${targets.map((t) => `${t.type} ${t.url}`).join(', ') || '(none)'}`);
    process.exit(2);
  }
  console.log(`→ attached to ${target.url}`);

  const cdp = await connect(target.webSocketDebuggerUrl);
  const errors = [];
  cdp.on((msg) => {
    if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
      errors.push(`console.error: ${msg.params.args.map(describe).join(' ')}`);
    } else if (msg.method === 'Runtime.exceptionThrown') {
      const d = msg.params.exceptionDetails;
      errors.push(`exception: ${d.exception?.description ?? d.text}`);
    } else if (msg.method === 'Log.entryAdded' && msg.params.entry.level === 'error') {
      errors.push(`log: ${msg.params.entry.text}`);
    }
  });
  await cdp.send('Runtime.enable');
  await cdp.send('Log.enable'); // replays entries logged before we attached
  await sleep(500);
  const startupErrors = errors.splice(0);

  // CDP answers as soon as the WebView exists, which is BEFORE Svelte has mounted — attaching the
  // moment the port opens found an empty sidebar and reported the app broken. Wait for the nav to
  // appear instead of assuming it: a smoke test that cries wolf is one nobody reads.
  const listTabs = `[...document.querySelectorAll(${JSON.stringify(NAV)})].map((b) => b.dataset.tab).filter(Boolean)`;
  let tabs = [];
  const mountBy = Date.now() + MOUNT_MS;
  for (;;) {
    tabs = (await evaluate(cdp, listTabs)) ?? [];
    if (tabs.length) break;
    if (Date.now() > mountBy) {
      console.error(`✗ No sidebar tabs after ${MOUNT_MS / 1000}s (selector ${NAV}).`);
      console.error(`  document.readyState=${await evaluate(cdp, 'document.readyState')}`);
      console.error(`  body text: ${JSON.stringify(((await evaluate(cdp, 'document.body.innerText')) ?? '').slice(0, 200))}`);
      if (errors.length) console.error(`  console: ${errors[0].split('\n')[0]}`);
      cdp.close();
      process.exit(1);
    }
    await sleep(250);
  }

  const failures = startupErrors.length ? [{ tab: '(startup)', errors: startupErrors }] : [];
  for (const tab of tabs) {
    let state;
    try {
      state = await openTab(cdp, tab);
    } catch (e) {
      failures.push({ tab, errors: [e.message, ...errors.splice(0)] });
      console.log(`  ✗ ${tab} — ${e.message}`);
      continue;
    }
    // splice, not slice: drain the buffer every iteration so an error that lands BETWEEN two tab
    // windows is still attributed to one of them instead of being silently skipped.
    const problems = errors.splice(0);
    if (!state.active) problems.push('tab never became active after click');
    if (state.text.length < MIN_TEXT) {
      problems.push(`main region rendered ${state.text.length} chars (< ${MIN_TEXT}) — blank tab`);
    }
    if (problems.length) {
      failures.push({ tab, errors: problems });
      console.log(`  ✗ ${tab}`);
    } else {
      console.log(`  ✓ ${tab} (${state.text.length} chars)`);
    }
  }
  cdp.close();

  if (failures.length) {
    console.error(`\n✗ SMOKE FAILED — ${failures.length} of ${tabs.length} tabs broken:`);
    for (const f of failures) {
      console.error(`\n  [${f.tab}]`);
      for (const e of f.errors) console.error(`    ${e.split('\n')[0]}`);
    }
    process.exit(1);
  }
  console.log(`\n✓ SMOKE PASSED — ${tabs.length} tabs rendered, 0 console errors.`);
}

main().catch((e) => {
  console.error(`✗ smoke crashed: ${e.stack ?? e.message}`);
  process.exit(1);
});
