// Subagents tab state, moved verbatim out of +page.svelte following the MatrixState pattern.
// Like the Environments tab, every action here is a direct native invoke — no global run lock, no
// console-log stream — so the whole tab could leave the orchestrator intact.
//
// Module singleton for the same reason as EnvState: SubagentsTab is remounted by the {#key active}
// tab chain, and `loaded` must keep the page's old load-once-per-session behaviour.
import type { AgentInfo } from '$lib/ipc';
import { listAgents, saveAgent, deleteAgent } from '$lib/ipc';
import { pushToast } from '$lib/toast.svelte';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';

export type AgentDraft = {
  name: string;
  description: string;
  model: string;
  tools: string;
  prompt: string;
  path?: string;
};

export class AgentsState {
  data = $state<AgentInfo[] | null>(null);
  /** First-open gate (was `agentsLoaded` in +page) — survives the tab's remount. */
  loaded = $state(false);
  /** Drives the page's refresh overlay + sidebar spinner for this tab. */
  loading = $state(false);
  /** $lib/confirmGate is the ONE confirm mechanism; the tab hands the page's gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};

  async load() {
    try {
      this.data = await listAgents();
    } catch {
      this.data = null;
    }
  }
  /** Lazy first-open load — cheap native dir read, no script spawn. */
  loadOnce() {
    if (this.loaded) return;
    this.loaded = true;
    this.loading = true;
    void this.load().finally(() => (this.loading = false));
  }

  /** Throws on failure so SubagentsTab keeps the editor open (nothing typed is lost). */
  async save(a: AgentDraft) {
    try {
      await saveAgent(a);
    } catch (e) {
      pushToast({ kind: 'error', title: t('agents.saveError'), detail: String(e) });
      throw e;
    }
    pushToast({
      kind: 'success',
      title: a.path ? t('agents.savedEdit', { name: a.name }) : t('agents.savedNew', { name: a.name })
    });
    await this.load();
  }

  remove(a: AgentInfo) {
    this.askConfirm({
      title: t('agents.deleteTitle'),
      message: t('agents.deleteConfirm', { name: a.name }),
      confirmLabel: t('agents.delete'),
      action: async () => {
        try {
          await deleteAgent(a.path);
          pushToast({ kind: 'success', title: t('agents.deleted', { name: a.name }) });
          await this.load();
        } catch (e) {
          pushToast({ kind: 'error', title: t('agents.deleteError'), detail: String(e) });
        }
      },
      danger: true
    });
  }
}

export const agentsState = new AgentsState();
