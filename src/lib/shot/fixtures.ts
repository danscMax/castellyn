// DEV-only screenshot fixtures: realistic, fully-public demo data returned by the mocked Tauri
// IPC layer so the SvelteKit frontend renders populated tabs in a plain browser (no backend).
// Wired in +layout.ts behind `import.meta.env.DEV && ?shot` — never reaches a release build.
// Recapture harness: tools/shoot.py. See commit 76c4d97 for the original (throwaway) approach.

/* eslint-disable @typescript-eslint/no-explicit-any */

const PROFILES = [
  { name: 'main', description: 'Primary daily driver', color: '#3b82f6' },
  { name: 'work', description: 'Client work — separate login', color: '#22c55e' },
  { name: 'research', description: 'Experiments & long-context', color: '#a855f7' },
  { name: 'opus-max', description: 'Opus 4.8 1M context', color: '#f59e0b' }
];

const SHARED = ['skills', 'agents', 'commands', 'plugins', 'projects'];

const profileInfo = (p: (typeof PROFILES)[number]) => ({
  name: p.name,
  description: p.description,
  color: p.color,
  exists: true,
  credentialsPresent: true,
  settingsPresent: true,
  sharedLinks: Object.fromEntries(SHARED.map((s) => [s, `~/.claude/${s}`])),
  linksIntact: true
});

const usage: Record<string, any> = {
  main: { fiveHourPct: 42, sevenDayPct: 61, fiveHourResetsAt: '2026-06-25T18:00:00Z', sevenDayResetsAt: '2026-06-30T00:00:00Z' },
  work: { fiveHourPct: 12, sevenDayPct: 34, fiveHourResetsAt: '2026-06-25T17:30:00Z', sevenDayResetsAt: '2026-06-29T00:00:00Z' },
  research: { fiveHourPct: 78, sevenDayPct: 55, fiveHourResetsAt: '2026-06-25T19:15:00Z', sevenDayResetsAt: '2026-07-01T00:00:00Z' },
  'opus-max': { fiveHourPct: 5, sevenDayPct: 22, fiveHourResetsAt: '2026-06-25T16:45:00Z', sevenDayResetsAt: '2026-06-28T00:00:00Z' }
};

const forkStatus = {
  schemaVersion: 1,
  status: 'changes',
  timestamp: '2026-06-25T09:12:00Z',
  generatedAt: '2026-06-25T09:12:00Z',
  mode: 'check',
  ghAvailable: true,
  durationSec: 7.4,
  summary: { repos: 4, merged: 1, open: 2, conflict: 1, needHands: 1 },
  repos: [
    {
      Name: 'claude-code', Path: 'E:\\Scripts\\External\\claude-code',
      upstream: 'https://github.com/anthropics/claude-code', fork: 'https://github.com/danscMax/claude-code',
      forkOwnerRepo: 'danscMax/claude-code', parentOwnerRepo: 'anthropics/claude-code',
      defaultBranch: 'main', behindBy: 12, defaultAhead: 0, ffSafe: true, dirty: false, untracked: false,
      midOp: false, opName: null, detached: false, currentBranch: 'main', isOwn: false, rolesGuessed: false,
      wipLocal: { behindBy: 0, mergedPatches: 1, uniquePatches: 2 },
      upstreamUpdated: '2026-06-24T20:00:00Z', upstreamArchived: false, upstreamDefaultBranch: 'main',
      branches: [
        { name: 'feat/statusline-themes', prNumber: 142, prState: 'OPEN', url: 'https://github.com/anthropics/claude-code/pull/142', outcome: null, conflictFiles: null, aheadOfUpstream: 3, cherryPlus: 3, divergedFromForkAhead: 0, checks: 'pass', action: null },
        { name: 'fix/win-pty-flicker', prNumber: 139, prState: 'MERGED', url: 'https://github.com/anthropics/claude-code/pull/139', outcome: null, conflictFiles: null, aheadOfUpstream: 0, cherryPlus: 0, divergedFromForkAhead: 0, checks: 'pass', action: 'delete' }
      ],
      Skipped: null
    },
    {
      Name: 'tauri', Path: 'E:\\Scripts\\External\\tauri',
      upstream: 'https://github.com/tauri-apps/tauri', fork: 'https://github.com/danscMax/tauri',
      forkOwnerRepo: 'danscMax/tauri', parentOwnerRepo: 'tauri-apps/tauri',
      defaultBranch: 'dev', behindBy: 3, defaultAhead: 0, ffSafe: true, dirty: false, untracked: false,
      midOp: false, opName: null, detached: false, currentBranch: 'dev', isOwn: false, rolesGuessed: false,
      wipLocal: null, upstreamUpdated: '2026-06-25T06:00:00Z', upstreamArchived: false, upstreamDefaultBranch: 'dev',
      branches: [
        { name: 'feat/no-window-spawn', prNumber: 88, prState: 'OPEN', url: 'https://github.com/tauri-apps/tauri/pull/88', outcome: null, conflictFiles: null, aheadOfUpstream: 1, cherryPlus: 1, divergedFromForkAhead: 0, checks: 'running', action: null }
      ],
      Skipped: null
    },
    {
      Name: 'opencode', Path: 'E:\\Scripts\\External\\opencode',
      upstream: 'https://github.com/sst/opencode', fork: 'https://github.com/danscMax/opencode',
      forkOwnerRepo: 'danscMax/opencode', parentOwnerRepo: 'sst/opencode',
      defaultBranch: 'main', behindBy: 0, defaultAhead: 0, ffSafe: true, dirty: false, untracked: false,
      midOp: false, opName: null, detached: false, currentBranch: 'feat/profile-env', isOwn: false, rolesGuessed: false,
      wipLocal: null, upstreamUpdated: '2026-06-22T11:00:00Z', upstreamArchived: false, upstreamDefaultBranch: 'main',
      branches: [
        { name: 'feat/profile-env', prNumber: 51, prState: 'OPEN', url: 'https://github.com/sst/opencode/pull/51', outcome: 'conflict', conflictFiles: ['packages/core/src/config.ts', 'packages/cli/src/run.ts'], aheadOfUpstream: 4, cherryPlus: 2, divergedFromForkAhead: 2, checks: 'fail', action: 'rebase' }
      ],
      Skipped: null
    },
    {
      Name: 'fork-updater', Path: 'E:\\Scripts\\fork-updater',
      upstream: null, fork: 'https://github.com/danscMax/fork-updater',
      forkOwnerRepo: 'danscMax/fork-updater', parentOwnerRepo: null,
      defaultBranch: 'main', behindBy: null, defaultAhead: 0, ffSafe: true, dirty: false, untracked: false,
      midOp: false, opName: null, detached: false, currentBranch: 'main', isOwn: true, rolesGuessed: false,
      wipLocal: null, upstreamUpdated: null, upstreamArchived: null, upstreamDefaultBranch: null,
      branches: [], Skipped: null
    }
  ]
};

const githubRepos = [
  { owner: 'danscMax', name: 'castellyn', nameWithOwner: 'danscMax/castellyn', isPrivate: false, isFork: false, isArchived: false, url: 'https://github.com/danscMax/castellyn', updatedAt: '2026-06-25T09:00:00Z', description: 'Native control center for a local AI-coding dev environment', language: 'Rust', stars: 0 },
  { owner: 'danscMax', name: 'rtk', nameWithOwner: 'danscMax/rtk', isPrivate: false, isFork: false, isArchived: false, url: 'https://github.com/danscMax/rtk', updatedAt: '2026-06-20T12:00:00Z', description: 'Rust Token Killer — token-optimized CLI proxy', language: 'Rust', stars: 3 }
];

// Fake Claude-Code terminal screens streamed into a pane's PTY Channel so the Sessions grid shows
// real-looking running terminals (the flagship parallel-sessions feature) instead of an empty state.
const E = '\x1b['; // CSI
// Lines per fake session screen; streamed one-per-frame so xterm's WebGL renderer paints every row
// (a single synchronous write only repaints the final frame, leaving earlier rows blank).
const SESSION_SCREENS: string[][] = [
  [
    `${E}38;5;39m✻ Welcome to Claude Code${E}0m  ${E}90m·  profile ${E}0m${E}38;5;39mmain${E}0m`,
    `${E}90m  ~/projects/castellyn · claude-opus-4-8 (1M context)${E}0m`,
    ``,
    `${E}32m›${E}0m make the status-envelope writer DRY across all scripts`,
    `${E}90m  ⎿ Read tools/ScriptKit.ps1 (118 lines)${E}0m`,
    `${E}90m  ⎿ Updated Write-StatusJson  ${E}0m${E}32m+14 ${E}31m−9${E}0m`,
    `${E}90m  ⎿ Ran npm run check  ${E}0m${E}32m✓ 0 errors${E}0m`,
    ``,
    `${E}38;5;39m✻${E}0m Done — every script emits the envelope via one helper.`,
    ``,
    `${E}32m›${E}0m ▍`
  ],
  [
    `${E}38;5;39m✻ Welcome to Claude Code${E}0m  ${E}90m·  profile ${E}0m${E}38;5;36mwork${E}0m`,
    `${E}90m  ~/clients/acme-api · claude-sonnet-4-6${E}0m`,
    ``,
    `${E}32m›${E}0m add pagination to the orders endpoint`,
    `${E}90m  ⎿ Read src/routes/orders.ts (84 lines)${E}0m`,
    `${E}90m  ⎿ Added ?cursor / ?limit + tests  ${E}0m${E}32m+61 ${E}31m−4${E}0m`,
    `${E}90m  ⎿ Ran pytest  ${E}0m${E}32m✓ 23 passed${E}0m`,
    ``,
    `${E}32m›${E}0m ▍`
  ],
  [
    `${E}38;5;39m✻ Welcome to Claude Code${E}0m  ${E}90m·  profile ${E}0m${E}38;5;141mresearch${E}0m`,
    `${E}90m  ~/lab/rl-sweep · claude-opus-4-8 · ${E}0m${E}33mlean${E}0m`,
    ``,
    `${E}32m›${E}0m summarize today's eval runs and flag regressions`,
    `${E}90m  ⎿ Scanned 42 run logs${E}0m`,
    `${E}90m  ⎿ 3 configs beat baseline · 1 regression (seed 7)${E}0m`,
    ``,
    `${E}32m›${E}0m ▍`
  ]
];
let spawnSeq = 0;

const handlers: Record<string, (args: any) => any> = {
  session_spawn: (a) => {
    const chan = a?.onData;
    const lines = SESSION_SCREENS[spawnSeq++ % SESSION_SCREENS.length];
    if (chan && typeof chan.onmessage === 'function') {
      const send = (s: string) => { try { chan.onmessage(new TextEncoder().encode(s).buffer); } catch { /* */ } };
      // Wait for the fit-addon to size the pane, reset screen+scrollback, then stream one line per
      // frame so the WebGL renderer paints every row (a single bulk write only repaints the last).
      let t = 900;
      setTimeout(() => send(`${E}2J${E}3J${E}H`), t);
      for (const ln of lines) { t += 70; const s = ln; setTimeout(() => send(s + '\r\n'), t); }
    }
    return `sess-${spawnSeq}`;
  },
  session_attach: () => 1,
  session_list: () => [],
  // --- init / settings ---
  read_config: () => ({ scriptsRoot: 'E:\\Scripts', startHidden: false, closeToTray: true, fetchTimeoutSec: 30, ghTimeoutSec: 20, toggleHotkey: 'Ctrl+Shift+H', shortcuts: { toggle_window: 'Ctrl+Shift+H' }, language: 'en' }),
  read_shortcuts: () => ({ toggle_window: 'Ctrl+Shift+H' }),
  set_shortcuts: () => 0,
  app_paths: () => ({ scriptsRoot: 'E:\\Scripts', configPath: '%APPDATA%\\castellyn\\config.json', exe: 'castellyn.exe', stackPath: 'E:\\Scripts\\llm-stack\\stack.json', backupDir: 'E:\\Scripts\\!Настройки и MCP\\ClaudeProfiles\\Backups' }),
  get_autostart: () => true,
  set_language: () => 0,
  list_components: () => ([
    { id: 'all', name: 'Everything', group: 'orchestrator', lastJson: 'all.last.json', supportsApply: true },
    { id: 'plugins', name: 'Plugins', group: 'claude', lastJson: 'plugins.last.json', supportsApply: true },
    { id: 'forks', name: 'GitHub forks', group: 'git', lastJson: 'forks.last.json', supportsApply: true },
    { id: 'rtk', name: 'RTK', group: 'tools', lastJson: 'rtk.last.json', supportsApply: true },
    { id: 'speckit', name: 'SpecKit', group: 'tools', lastJson: 'speckit.last.json', supportsApply: true },
    { id: 'opencode', name: 'opencode', group: 'agents', lastJson: 'opencode.last.json', supportsApply: true },
    { id: 'cargo', name: 'Cargo bins', group: 'tools', lastJson: 'cargo.last.json', supportsApply: true }
  ]),
  read_status: (a) => {
    const p = typeof a?.path === 'string' ? a.path.toLowerCase() : '';
    if (p.includes('fork')) return forkStatus;
    // Last-run envelopes for the Home recent-runs feed (schema = tools/ScriptKit.ps1 Write-StatusJson).
    const env = (component: string, status: string, timestamp: string, durationSec: number, changed: number, summary: string) =>
      ({ schemaVersion: 1, component, status, timestamp, mode: 'apply', durationSec, counts: { changed, failed: 0, total: changed }, summary });
    if (p.includes('plugins')) return env('plugins', 'ok', '2026-06-25T08:40:00Z', 41, 3, '3 plugins updated, 2 already current');
    if (p.includes('rtk')) return env('rtk', 'ok', '2026-06-25T08:35:00Z', 12, 1, 'rtk 0.9.4 -> 0.9.5');
    if (p.includes('opencode')) return env('opencode', 'changes', '2026-06-25T08:30:00Z', 8, 1, 'update available: 1.17.9 -> 1.17.11');
    if (p.includes('cargo')) return env('cargo', 'ok', '2026-06-24T03:10:00Z', 95, 0, 'all 12 binaries current');
    return null;
  },

  // --- Profiles ---
  read_profiles: () => ({
    generatedAt: '2026-06-25T09:00:00Z', isAdmin: true,
    profiles: PROFILES.map(profileInfo),
    syncConflicts: { count: 0, files: [] },
    backup: { lastRun: '2026-06-25T03:00:00Z', lastSnapshot: '2026-06-25_0300', ageHours: 6, stale: false }
  }),
  read_profiles_config: () => ({ schemaVersion: 1, sharedFoldersDefault: SHARED, profiles: PROFILES.map((p) => ({ name: p.name, color: p.color, description: p.description, linkedFolders: SHARED })) }),
  read_launch_config: () => ({
    profiles: PROFILES.map((p) => ({ name: p.name, mode: p.name === 'research' ? 'lean' : 'full', mcp: ['context7', 'serena'], claudeMd: true, tokenAuth: p.name === 'work' })),
    availableMcp: ['context7', 'serena', 'playwright', 'chrome-devtools']
  }),
  read_profile_usage: (a) => usage[a?.profile] ?? null,

  // Ф2.5 matrix — 10 profiles; one has a proxy, one is 5/7 folders with a real-data mismatch.
  read_profile_matrix: () => {
    const F7 = ['agents', 'commands', 'hooks', 'plugins', 'skills', 'projects', 'history.jsonl'];
    const allLinked = F7.map((name) => ({ name, desired: true, actual: 'linked' as 'linked' | 'real' | 'missing' }));
    type PState = 'on' | 'off' | 'unset';
    const mkPlugins = (...states: PState[]) =>
      ['superpowers@2.4.1', 'ponytail@1.1.0', 'speckit@0.9.3', 'drywall@0.3.0'].map((id, i) => ({ id, state: states[i] ?? 'on' }));
    const CANON = ['context7', 'serena', 'playwright', 'chrome-devtools', 'claude-in-chrome'];
    const fullMcp = { canon: CANON, deployed: CANON, extras: [] as string[] };
    const mk = (
      name: string,
      color: string,
      description: string,
      baseUrl = '',
      proxy = '',
      folders = allLinked,
      plugins = mkPlugins('on', 'on', 'on', 'on'),
      mcp = fullMcp
    ) => ({
      name,
      color,
      description,
      provider: { baseUrl, model: baseUrl ? 'deepseek-chat' : '', smallModel: baseUrl ? 'deepseek-chat' : '', hasToken: !!baseUrl },
      proxy,
      folders,
      plugins,
      mcp
    });
    return [
      mk('ccmy', 'Cyan', 'Personal'),
      mk('cc1', 'Green', 'Med1', '', '', allLinked, mkPlugins('on', 'on', 'unset', 'on')),
      mk('cc2', 'Green', 'Med2'),
      // cc3: extra (non-canon) MCP server deployed + a socks5 proxy.
      mk('cc3', 'Yellow', '3', '', 'socks5://127.0.0.1:1080', allLinked, mkPlugins('on', 'on', 'on', 'on'), {
        canon: CANON,
        deployed: CANON,
        extras: ['filesystem']
      }),
      mk('ccfree', 'Magenta', 'Free tier', 'https://api.deepseek.com', '', [
        { name: 'agents', desired: true, actual: 'linked' },
        { name: 'commands', desired: true, actual: 'linked' },
        { name: 'hooks', desired: true, actual: 'linked' },
        { name: 'plugins', desired: true, actual: 'real' }, // holds real data instead of a link
        { name: 'skills', desired: true, actual: 'linked' },
        { name: 'projects', desired: false, actual: 'missing' },
        { name: 'history.jsonl', desired: false, actual: 'missing' }
      ], mkPlugins('on', 'off', 'on', 'unset')),
      mk('cc4', 'Blue', 'Med4'),
      mk('cc5', 'DarkGreen', 'Med5'),
      mk('cc6', 'DarkCyan', 'Med6'),
      // cctest: two canon MCP servers not yet deployed (missing).
      mk('cctest', 'Gray', 'Throwaway', '', '', allLinked, mkPlugins('on', 'on', 'on', 'on'), {
        canon: CANON,
        deployed: CANON.slice(0, 3),
        extras: []
      }),
      mk('research', 'DarkMagenta', 'Long-context', 'http://127.0.0.1:3456')
    ];
  },
  set_profile_proxy: () => null,
  set_profile_folders: () => [],
  set_profile_plugins: () => null,
  // Orphan profile dirs (Adopt/Delete section) — none in the demo shot.
  read_orphan_profiles: () => [],

  // --- Providers / engines / stack ---
  read_engines: () => ([
    { id: 'gateway', name: 'freellmapi gateway', baseUrl: 'http://127.0.0.1:8787', protocol: 'openai', port: 8787, dashboardUrl: 'http://127.0.0.1:8787', hasCommand: true, router: false, installed: null, running: true },
    { id: 'ccr', name: 'claude-code-router', baseUrl: 'http://127.0.0.1:3456', protocol: 'anthropic', port: 3456, dashboardUrl: '', hasCommand: true, router: true, installed: true, running: true }
  ]),
  read_stack: () => ([
    { id: 'gateway', name: 'freellmapi gateway', group: 'core', port: 8787, protocol: 'openai+anthropic', dashboard: 'http://127.0.0.1:8787', dir: 'E:\\Scripts\\llm-stack\\gateway', enabled: true, running: true },
    { id: 'ccr', name: 'claude-code-router', group: 'router', port: 3456, protocol: 'anthropic', dashboard: '', dir: 'E:\\Scripts\\llm-stack\\ccr', enabled: true, running: true }
  ]),
  read_stack_health: () => ([
    { id: 'gateway', name: 'freellmapi gateway', group: 'core', port: 8787, enabled: true, portOpen: true, healthy: true },
    { id: 'ccr', name: 'claude-code-router', group: 'router', port: 3456, enabled: true, portOpen: true, healthy: null }
  ]),
  read_stack_procs: () => ([{ port: 8787, pid: 18244, uptimeSec: 13620 }, { port: 3456, pid: 9012, uptimeSec: 13580 }]),
  read_providers: () => ([
    { name: 'main', baseUrl: 'https://api.anthropic.com', model: 'claude-opus-4-8', smallModel: 'claude-haiku-4-5', hasToken: true },
    { name: 'research', baseUrl: 'http://127.0.0.1:3456', model: 'deepseek-v3', smallModel: 'deepseek-v3', hasToken: true }
  ]),
  list_my_providers: () => ([
    { id: 'deepseek', name: 'DeepSeek', baseUrl: 'https://api.deepseek.com', protocol: 'openai', authScheme: 'bearer', model: 'deepseek-chat', smallModel: 'deepseek-chat', connectVia: 'direct', targetProfile: 'research', balanceUrl: 'https://platform.deepseek.com', createdAt: '2026-05-01T00:00:00Z', hasKey: true, keyCount: 1, activeKey: 0 },
    { id: 'glm', name: 'GLM (Zhipu)', baseUrl: 'https://open.bigmodel.cn/api/paas/v4', protocol: 'openai', authScheme: 'bearer', model: 'glm-4-plus', smallModel: 'glm-4-flash', connectVia: 'freellmapi', targetProfile: 'work', balanceUrl: '', createdAt: '2026-05-10T00:00:00Z', hasKey: true, keyCount: 3, activeKey: 1 }
  ]),
  check_provider_balance: () => ({ ok: true, amount: 24.7, currency: 'USD', detail: 'balance ok' }),
  read_opencode: () => ({ installed: true, model: 'anthropic/claude-opus-4-8', providers: [{ id: 'anthropic', name: 'Anthropic', baseUrl: 'https://api.anthropic.com', hasKey: true }, { id: 'deepseek', name: 'DeepSeek', baseUrl: 'https://api.deepseek.com', hasKey: true }] }),
  freellmapi_auth_status: () => ({ hasEmail: true, hasToken: true }),

  // --- MCP ---
  read_mcp: () => ({
    profiles: PROFILES.map((p) => p.name),
    source: [
      { name: 'context7', command: 'npx -y @upstash/context7-mcp', deployedIn: ['main', 'work', 'research', 'opus-max'] },
      { name: 'serena', command: 'uvx --from git+https://github.com/oraios/serena serena', deployedIn: ['main', 'research', 'opus-max'] },
      { name: 'playwright', command: 'npx @playwright/mcp@latest', deployedIn: ['main', 'work'] },
      { name: 'chrome-devtools', command: 'npx chrome-devtools-mcp@latest', deployedIn: ['main'] },
      { name: 'claude-in-chrome', command: 'node ./claude-in-chrome/server.js', deployedIn: ['main', 'work'] }
    ],
    extras: [{ name: 'filesystem', presentIn: ['research'] }]
  }),

  // --- Sync ---
  read_sync: () => ({
    generatedAt: '2026-06-25T09:00:00Z',
    items: { history: true, projects: true, skills: true, agents: true, commands: true, keybindings: false },
    stignoreMatches: true, stignoreExists: true,
    syncthing: { available: true, version: 'v1.27.10', folderId: 'claude-sync', folderLabel: 'Claude', folderShared: true, state: 'idle', globalBytes: 2_415_919_104, needBytes: 0, completion: 100, connectedDevices: 2 }
  }),
  read_config_drift: () => ({ generatedAt: '2026-06-25T09:00:00Z', drifted: 0, unlinked: 0, ok: true, items: [
    { name: 'settings.json', state: 'linked' }, { name: 'CLAUDE.md', state: 'linked' }, { name: 'mcp.json', state: 'linked' }
  ] }),

  // --- Plugins & skills ---
  list_plugins: () => ([
    { id: 'superpowers', version: '2.4.1', scope: 'user', enabled: true, installedAt: '2026-03-02', lastUpdated: '2026-06-20', description: 'Skill discovery + disciplined workflows', mine: false },
    { id: 'ponytail', version: '1.1.0', scope: 'user', enabled: true, installedAt: '2026-04-11', lastUpdated: '2026-06-18', description: 'Laziest-solution-that-works engineering mode', mine: false },
    { id: 'speckit', version: '0.9.3', scope: 'user', enabled: true, installedAt: '2026-02-19', lastUpdated: '2026-06-15', description: 'Spec-driven development workflow', mine: false },
    { id: 'drywall', version: '0.3.0', scope: 'user', enabled: false, installedAt: '2026-05-01', lastUpdated: '2026-05-30', description: 'Duplicate-code detection (jscpd)', mine: false },
    { id: 'max', version: '3.0.0', scope: 'user', enabled: true, installedAt: '2026-01-15', lastUpdated: '2026-06-24', description: 'Multi-agent audits, reviews, smoke tests', mine: true },
    // Managed-policy quarantined plugin — renders the 🔒 unblock button instead of a toggle.
    { id: 'serena', version: '0.2.0', scope: 'managed', enabled: false, managedPolicy: false, installedAt: '2026-05-10', lastUpdated: '2026-06-01', description: 'Semantic code retrieval MCP', mine: false }
  ]),
  read_codex_profiles: () => (['work', 'personal']),
  list_skills: () => ([
    { name: 'brainstorming', description: 'Explore intent before building', version: '1.0', dir: '~/.claude/skills/brainstorming', source: 'plugin:superpowers', mine: false },
    { name: 'systematic-debugging', description: 'Root-cause before fixing', version: '1.0', dir: '~/.claude/skills/systematic-debugging', source: 'plugin:superpowers', mine: false },
    { name: 'tauri-v2', description: 'Tauri v2 app development', version: '1.0', dir: '~/.claude/skills/tauri-v2', source: 'default', mine: false },
    { name: 'powershell-cyrillic', description: 'PowerShell with Cyrillic paths', version: '1.0', dir: '~/.claude/skills/powershell-cyrillic', source: 'own', mine: true },
    { name: 'claude-backup', description: 'Backup/restore the Claude setup', version: '1.2', dir: '~/.claude/skills/claude-backup', source: 'own', mine: true }
  ]),
  list_plugin_updates: () => ([{ id: 'superpowers', installed: '2.4.1', available: '2.5.0' }]),
  agent_status_hook_status: () => ({
    wired: ['.claude', '.claude-cc1', '.claude-cc2', '.claude-ccfree', '.claude-cctest'],
    unwired: []
  }),
  plugin_sync_status: () => ({
    wired: ['.claude', '.claude-cc1', '.claude-cc2', '.claude-ccfree'],
    unwired: ['.claude-cctest'],
    scriptInstalled: true,
    scriptVersion: 2
  }),
  // Ф1: stack-ownership drift card on Home — demo one fixable drift + two ok rows.
  read_stack_drift: () => ([
    { id: 'plugin_sync_file', state: 'drift', detail: 'on-disk hook is an external version (no Castellyn marker)', fix: 'plugin_sync' },
    { id: 'plugin_sync_wiring', state: 'ok', detail: 'every profile wired; managed settings clean', fix: null },
    { id: 'managed_settings', state: 'ok', detail: 'deployed matches source', fix: null },
    // Ф3: own-marketplace version alignment.
    { id: 'marketplace_versions', state: 'drift', detail: 'max@max-marketplace: installed 1.9.0 behind source 1.14.1 — update', fix: null }
  ]),
  run_managed_deploy: () => ({ id: 'managed_settings', state: 'ok', detail: 'deployed matches source', fix: null }),
  // Ф3: dual-manifest bump of an own-marketplace plugin.
  run_marketplace_bump: () => 0,
  // Ф2-GC: stack-garbage scan — stale versions + temp_git + .bak (deletable) + wrong-OS (report-only).
  read_gc_scan: () => ([
    { id: 'stale:thedotmack/claude-mem/13.8.0', category: 'stale_version', label: 'claude-mem 13.8.0 (thedotmack)', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\thedotmack\\claude-mem\\13.8.0', size_bytes: 521_000_000, deletable: true },
    { id: 'stale:danscmax/max/1.7.0', category: 'stale_version', label: 'max 1.7.0 (danscmax)', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\danscmax\\max\\1.7.0', size_bytes: 12_600_000, deletable: true },
    { id: 'tempgit:temp_git_a1b2c3', category: 'temp_git', label: 'temp_git_a1b2c3', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\temp_git_a1b2c3', size_bytes: 2_600_000, deletable: true },
    { id: 'bak:known_marketplaces.json.bak', category: 'bak', label: 'known_marketplaces.json.bak', path: 'C:\\Users\\User\\.claude\\plugins\\known_marketplaces.json.bak', size_bytes: 10_240, deletable: true },
    { id: 'wrongos:.claude', category: 'wrong_os', label: 'darwin/linux binaries (.claude)', path: 'C:\\Users\\User\\.claude\\plugins\\cache', size_bytes: 249_000_000, deletable: false }
  ]),
  run_gc_delete: (args) => ({ deleted: args?.ids ?? [], skipped: [], freed_bytes: 536_200_000 }),
  list_plugin_contents: () => ([
    {
      id: 'superpowers',
      skills: [
        { name: 'brainstorming', description: 'Explore user intent, requirements and design before implementation.', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\obra\\superpowers\\2.4.1\\skills\\brainstorming\\SKILL.md' },
        { name: 'systematic-debugging', description: 'Root-cause investigation before proposing fixes.', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\obra\\superpowers\\2.4.1\\skills\\systematic-debugging\\SKILL.md' },
        { name: 'test-driven-development', description: undefined, path: 'C:\\Users\\User\\.claude\\plugins\\cache\\obra\\superpowers\\2.4.1\\skills\\test-driven-development\\SKILL.md' }
      ],
      commands: [{ name: 'spec', description: 'Turn vague intent into a precise, executable spec in five phases.', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\obra\\superpowers\\2.4.1\\commands\\spec.md' }],
      agents: [
        { name: 'Plan', description: 'Software architect agent for designing implementation plans.', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\obra\\superpowers\\2.4.1\\agents\\plan.md' },
        { name: 'Explore', description: 'Read-only search agent for broad fan-out searches.', path: 'C:\\Users\\User\\.claude\\plugins\\cache\\obra\\superpowers\\2.4.1\\agents\\explore.md' }
      ]
    },
    {
      id: 'max',
      skills: [
        { name: 'max-dedup', description: 'Universal duplicate-implementation audit: finds functionally-equivalent implementations with drift and produces a severity-ranked consolidation report.', path: 'E:\\Scripts\\SettingsMCP\\ClaudeMarketplace\\plugins\\max\\skills\\max-dedup\\SKILL.md' },
        { name: 'max-modernize', description: 'Multi-agent codebase audit against current best practices with web-sourced conventions.', path: 'E:\\Scripts\\SettingsMCP\\ClaudeMarketplace\\plugins\\max\\skills\\max-modernize\\SKILL.md' }
      ],
      commands: [
        { name: 'audit', description: 'Гибридный мультиагентный аудит сервиса — static-анализ + runtime-тестирование в браузере + DA-валидация, отчёт с evidence.', path: 'E:\\Scripts\\SettingsMCP\\ClaudeMarketplace\\plugins\\max\\commands\\audit.md' },
        { name: 'review', description: 'Комплексное ревью кода — параллельные волны критиков и оптимизаторов.', path: 'E:\\Scripts\\SettingsMCP\\ClaudeMarketplace\\plugins\\max\\commands\\review.md' }
      ],
      agents: [{ name: 'review-critic', description: 'Comprehensive Code Critic: bugs, security issues, SOLID violations, error handling gaps.', path: 'E:\\Scripts\\SettingsMCP\\ClaudeMarketplace\\plugins\\max\\agents\\review-critic.md' }]
    }
  ]),

  // Onboarding checklist: a fresh machine with the settings tree synced but nothing deployed.
  read_onboarding: () => ([
    { id: 'prereq_git', state: 'ok', detail: 'C:\\Program Files\\Git\\cmd\\git.exe', fix: null },
    { id: 'prereq_node', state: 'ok', detail: 'C:\\Program Files\\nodejs\\node.exe', fix: null },
    { id: 'prereq_claude', state: 'ok', detail: 'C:\\Users\\User\\AppData\\Roaming\\npm\\claude.cmd', fix: null },
    { id: 'prereq_syncthing', state: 'ok', detail: 'C:\\Users\\User\\AppData\\Local\\Syncthing\\config.xml', fix: null },
    { id: 'tree', state: 'ok', detail: 'E:\\Scripts\\!Настройки и MCP\\ClaudeProfiles', fix: null },
    { id: 'junction', state: 'todo', detail: 'E:\\Scripts\\SettingsMCP missing', fix: 'junction' },
    { id: 'profiles', state: 'todo', detail: '0/10', fix: 'install_profiles' },
    { id: 'creds', state: 'todo', detail: 'C:\\Users\\User\\.claude\\.credentials.json missing — restore a backup or log in once', fix: 'backup_tab' },
    { id: 'mcp', state: 'todo', detail: '10 profile(s) missing canon servers', fix: 'mcp_deploy' },
    { id: 'managed', state: 'todo', detail: 'deployed file missing', fix: 'managed_deploy' },
    { id: 'syncthing', state: 'unknown', detail: '', fix: 'syncthing' },
    { id: 'verify', state: 'unknown', detail: '', fix: 'verify' }
  ]),

  // --- Sessions launcher ---
  read_ssh_hosts: () => ([
    { id: 'minipc', name: 'MiniPC', host: '192.168.1.42', port: 22, user: 'dev', keyPath: '~/.ssh/id_ed25519', remoteDir: '/home/dev/work', source: 'saved' },
    { id: 'vps', name: 'cloud-vps', host: 'vps.example.net', port: 22, user: 'ubuntu', keyPath: null, remoteDir: null, source: 'sshconfig' }
  ]),
  list_monitors: () => ([
    { index: 0, name: 'DELL U2720Q', x: 0, y: 0, width: 3840, height: 2160, scale: 1.5, primary: true },
    { index: 1, name: 'LG 27GL', x: 3840, y: 0, width: 2560, height: 1440, scale: 1, primary: false }
  ]),

  // --- Forks ---
  list_github_repos: () => githubRepos,
  run_forks: () => 0,

  // --- Backup ---
  list_backups: () => ({ snapshots: ['2026-06-25_0300', '2026-06-24_0300', '2026-06-23_0300'], weeklies: ['2026-06-22_weekly'], state: { lastRun: '2026-06-25T03:00:00Z', lastManifestHash: 'a1b2c3', lastWeekly: '2026-06-22_weekly', lastSnapshot: '2026-06-25_0300' } }),

  // --- Schedule ---
  read_schedules: () => ({ generatedAt: '2026-06-25T09:00:00Z', tasks: [
    { id: 'backup', label: 'Daily backup', tn: 'Castellyn-Backup', exists: true, enabled: true, time: '03:00', nextRun: '2026-06-26T03:00:00Z', lastRun: '2026-06-25T03:00:00Z', lastResult: 0, status: 'ok', ok: true, defaultTime: '03:00' }
  ] }),

  // --- Analytics ---
  read_freellmapi_analytics: () => ({ available: true, totals: { totalRequests: 18432, successRate: 99.2, totalInputTokens: 42_500_000, totalOutputTokens: 6_200_000, avgLatencyMs: 740, estimatedCostSavings: 312.5, firstRequestAt: '2026-05-01T00:00:00Z' }, perModel: [], series: [], stepSec: 3600 }),

  // --- Environments (cross-harness coverage + fan-out) ---
  read_environments: () => ([
    { id: 'claude', name: 'Claude Code', installed: true, configPath: '~/.claude/settings.json', skillsVisible: 172, totalSkills: 172, pluginSkillsVisible: true, shareableGap: 0, providers: 4, mcpServers: 5, rtk: true, rtkAvailable: true, configOk: true },
    { id: 'opencode', name: 'OpenCode', installed: true, configPath: '~/.config/opencode/opencode.json', skillsVisible: 170, totalSkills: 172, pluginSkillsVisible: true, shareableGap: 0, providers: 3, mcpServers: 5, rtk: true, rtkAvailable: true, configOk: true },
    { id: 'codex', name: 'Codex', installed: true, configPath: '~/.codex/config.toml', skillsVisible: 169, totalSkills: 172, pluginSkillsVisible: false, shareableGap: 3, providers: 1, mcpServers: 4, rtk: false, rtkAvailable: false, configOk: true },
    { id: 'zcode', name: 'zcode', installed: false, configPath: '', skillsVisible: 0, totalSkills: 172, pluginSkillsVisible: false, shareableGap: 0, providers: 0, mcpServers: 0, rtk: false, rtkAvailable: false, configOk: true }
  ]),
  read_skill_matrix: () => ([
    { name: 'brainstorming', claude: true, opencode: true, codex: true, shareable: false },
    { name: 'systematic-debugging', claude: true, opencode: true, codex: true, shareable: false },
    { name: 'tauri-v2', claude: true, opencode: true, codex: false, shareable: true },
    { name: 'powershell-cyrillic', claude: true, opencode: true, codex: false, shareable: true },
    { name: 'claude-backup', claude: true, opencode: false, codex: false, shareable: true }
  ])
};

// Returns the fixture for a command, or a benign default so any unlisted command never throws.
export function fixtureFor(cmd: string, args: any): any {
  if (cmd in handlers) return handlers[cmd](args);
  // sensible empty defaults by shape of the command name
  if (cmd.startsWith('list_') || cmd.startsWith('read_')) return null;
  if (cmd.startsWith('run_') || cmd.startsWith('measure_')) return 0;
  return null;
}
