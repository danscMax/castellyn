// MCP tab state, moved verbatim out of +page.svelte following the EnvState/AgentsState pattern.
//
// Unlike Environments/Subagents this tab has one action that spawns a STREAMED run (deploy), and
// the global run lock + console log stay in +page — so the page hands its lock entry point in as
// `spawn`, exactly the way it hands the confirm gate in as `askConfirm`. Everything else here is a
// direct native invoke.
//
// Module singleton, and NOT lazily loaded: `data` feeds the command palette and `tick` drives the
// profile matrix's mcp re-read, so +page loads it on mount regardless of which tab is open.
import type { McpStatus } from '$lib/ipc';
import { readMcp, runMcp, mcpUpsertServer, mcpRemoveServer, mcpRemoveExtra } from '$lib/ipc';
import { pushToast } from '$lib/toast.svelte';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';
import type { SpawnRun } from '$lib/runSpawn';

export class McpState {
  data = $state<McpStatus | null>(null);
  /** Bumped on every reload → tells the profile matrix to re-read its mcp facts (deploy/remove). */
  tick = $state(0);
  /** $lib/confirmGate is the ONE confirm mechanism; +page hands its gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};
  /** The global run lock lives in +page; this is its entry point. */
  spawn: SpawnRun = () => false;
  /** +page's toastErr — logs the failure to the console AND toasts it. */
  toastErr: (e: unknown, title?: string) => void = () => {};

  async load() {
    try {
      this.data = await readMcp();
    } catch {
      this.data = null;
    }
    this.tick++;
  }

  private start(only?: string[]) {
    this.spawn('mcp', t('page.mcp_log'), () => runMcp('deploy', only));
  }

  // No arg → deploy to all profiles (confirm). A profile name or array → deploy just those.
  deploy(target?: string | string[]) {
    const profiles = target ? (Array.isArray(target) ? target : [target]) : null;
    // Gate broad writes (all profiles, or a multi-profile selection) behind a confirm; a single
    // explicit profile chip the user clicked deploys directly. Idempotent either way. (Was: only
    // the all-profiles path confirmed, while a multi-select bulk overwrote silently.)
    if (profiles && profiles.length === 1) {
      this.start(profiles);
      return;
    }
    this.askConfirm({
      title: t('page.confirm_mcp_title'),
      message: t('page.confirm_mcp_msg'),
      confirmLabel: t('page.confirm_mcp_btn'),
      action: () => this.start(profiles ?? undefined)
    });
  }

  // Canonical .mcp.json CRUD (native invokes; reload + toast, no run-log stream).
  async upsert(name: string, definition: string) {
    try {
      await mcpUpsertServer(name, definition);
      await this.load();
      pushToast({ kind: 'success', title: t('mcp.savedServer', { name }) });
    } catch (e) {
      // U4: rethrow so the form stays open with the error, instead of closing and losing the JSON.
      this.toastErr(e);
      throw e;
    }
  }
  removeServer(name: string) {
    this.askConfirm({
      title: t('page.confirm_mcp_remove_title'),
      message: t('page.confirm_mcp_remove_msg', { name }),
      confirmLabel: t('common.delete'),
      action: async () => {
        try {
          await mcpRemoveServer(name);
          await this.load();
          pushToast({ kind: 'success', title: t('mcp.removedServer', { name }) });
        } catch (e) {
          this.toastErr(e);
        }
      },
      danger: true
    });
  }
  removeExtra(name: string, profile: string) {
    this.askConfirm({
      title: t('page.confirm_mcp_extra_title'),
      message: t('page.confirm_mcp_extra_msg', { name, profile }),
      confirmLabel: t('common.delete'),
      action: async () => {
        try {
          await mcpRemoveExtra(name, profile);
          await this.load();
          pushToast({ kind: 'success', title: t('mcp.removedExtra', { name, profile }) });
        } catch (e) {
          this.toastErr(e);
        }
      },
      danger: true
    });
  }
}

export const mcpState = new McpState();
