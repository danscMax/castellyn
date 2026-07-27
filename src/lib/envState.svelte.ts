// Environments tab state, moved verbatim out of +page.svelte (the 3k-line orchestrator) following
// the MatrixState pattern: a $state-backed class that owns this tab's data and its own actions,
// instantiated once here and consumed by EnvironmentsTab. Nothing this tab does touches the global
// run lock or the console log — every action is a direct native invoke — which is exactly why it
// could move without dragging page-level machinery along.
//
// A module singleton (not `new` per component) because EnvironmentsTab is remounted by the
// {#key active} tab chain on every visit; a per-instance state would re-fetch each time, whereas
// the page's old `envsLoaded` flag loaded once per session. The `loaded` flag keeps that contract.
import type { EnvInfo, SkillRow } from '$lib/ipc';
import {
  readEnvironments,
  readSkillMatrix,
  shareSkills,
  shareCommands,
  runOpencodeRtk,
  runOpencodeMcp,
  runOpencodeProviders,
  runOpencodeInstructions,
  runCodexMcp,
  runCodexProviders,
  runCodexOmniroute
} from '$lib/ipc';
import { pushToast } from '$lib/toast.svelte';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';

export class EnvState {
  data = $state<EnvInfo[] | null>(null);
  matrix = $state<SkillRow[] | null>(null);
  /** First-open gate (was `envsLoaded` in +page) — survives the tab's remount. */
  loaded = $state(false);
  /** Drives the page's refresh overlay + sidebar spinner for this tab. */
  loading = $state(false);
  /** $lib/confirmGate is the ONE confirm mechanism; the tab hands the page's gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};

  async load() {
    try {
      this.data = await readEnvironments();
    } catch {
      this.data = null;
    }
  }
  /** Lazy first-open load — cheap native reads, no script spawn. */
  loadOnce() {
    if (this.loaded) return;
    this.loaded = true;
    this.loading = true;
    void this.load().finally(() => (this.loading = false));
  }

  async loadMatrix() {
    try {
      this.matrix = await readSkillMatrix();
    } catch (e) {
      // null means "still loading" to the tab (it renders the skeleton), so a FAILED read must not
      // reuse it — that showed a permanent spinner. Land on an empty matrix and say what went wrong.
      this.matrix = [];
      pushToast({ kind: 'error', title: t('environments.matrixLoadError'), detail: String(e) });
    }
  }

  // Enable/disable RTK command-rewriting for OpenCode (writes/removes a Windows-safe plugin).
  private async doRtk(enable: boolean) {
    try {
      await runOpencodeRtk(enable ? 'enable' : 'disable');
      pushToast({
        kind: 'success',
        title: enable ? t('environments.rtkEnabledToast') : t('environments.rtkDisabledToast')
      });
      await this.load();
    } catch (e) {
      pushToast({ kind: 'error', title: t('environments.rtkError'), detail: String(e) });
    }
  }
  onRtk(id: string, enable: boolean) {
    if (id !== 'opencode') return; // only OpenCode is wired today
    if (enable) {
      void this.doRtk(true);
    } else {
      // Disabling deletes the plugin file — gate it behind a confirm (#7).
      this.askConfirm({
        title: t('environments.rtkDisableTitle'),
        message: t('environments.rtkDisableConfirm'),
        confirmLabel: t('environments.rtkDisable'),
        action: () => void this.doRtk(false),
        danger: true
      });
    }
  }

  // One shape for every harness fan-out button: run, toast the count, refresh the cards.
  private async deployToHarness(run: () => Promise<number>, doneKey: string, errKey: string) {
    try {
      const n = await run();
      pushToast({ kind: 'success', title: t(doneKey, { n }) });
      await this.load();
    } catch (e) {
      pushToast({ kind: 'error', title: t(errKey), detail: String(e) });
    }
  }
  // Fan-outs write into another harness's config file — confirm the exact target first.
  private confirmFanout(target: string, run: () => void) {
    this.askConfirm({
      title: t('page.confirm_fanout_title', { target }),
      message: t('page.confirm_fanout_body', { target }),
      confirmLabel: t('page.confirm_mcp_btn'),
      action: run
    });
  }
  // Codex MCP is honest about partial failure: {added, failed}. Warn (not error) when some
  // servers failed but others landed; the ledger only advances when failed is empty.
  private async deployCodexMcp() {
    try {
      const res = await runCodexMcp();
      if (res.failed.length)
        pushToast({
          kind: 'warn',
          title: t('page.codex_mcp_partial', { added: res.added, failed: res.failed.length }),
          detail: res.failed[0]
        });
      else pushToast({ kind: 'success', title: t('environments.deployMcpDoneCodex', { n: res.added }) });
      await this.load();
    } catch (e) {
      pushToast({ kind: 'error', title: t('environments.deployMcpErrorCodex'), detail: String(e) });
    }
  }
  onDeployMcp(id: string) {
    if (id === 'opencode')
      this.confirmFanout('opencode.json', () =>
        void this.deployToHarness(runOpencodeMcp, 'environments.deployMcpDone', 'environments.deployMcpError'));
    else if (id === 'codex') this.confirmFanout('~/.codex/config.toml', () => void this.deployCodexMcp());
  }
  onDeployProviders(id: string) {
    if (id === 'opencode')
      this.confirmFanout('opencode.json', () =>
        void this.deployToHarness(runOpencodeProviders, 'environments.deployProvidersDone', 'environments.deployProvidersError'));
    else if (id === 'codex')
      // Not deployToHarness: the result is "was the key mirrored", which picks the toast text.
      this.confirmFanout('~/.codex/config.toml', () =>
        void (async () => {
          try {
            const keySet = await runCodexProviders();
            pushToast({
              kind: keySet ? 'success' : 'warn',
              title: t(keySet ? 'environments.connectGatewayDone' : 'environments.connectGatewayDoneNoKey')
            });
            await this.load();
          } catch (e) {
            pushToast({ kind: 'error', title: t('environments.connectGatewayError'), detail: String(e) });
          }
        })());
  }
  // OmniRoute -> Codex: writes [model_providers.omniroute] + the profile file; no key mirror
  // (OmniRoute owns its keys), so run_codex_omniroute always resolves to a plain success toast.
  onConnectOmniroute() {
    this.confirmFanout('~/.codex/config.toml', () =>
      void (async () => {
        try {
          await runCodexOmniroute();
          pushToast({ kind: 'success', title: t('environments.connectOmnirouteDone') });
          await this.load();
        } catch (e) {
          pushToast({ kind: 'error', title: t('environments.connectOmnirouteError'), detail: String(e) });
        }
      })());
  }
  onDeployInstructions(id: string) {
    if (id === 'opencode')
      this.confirmFanout('opencode.json', () =>
        void this.deployToHarness(runOpencodeInstructions, 'environments.deployInstrDone', 'environments.deployInstrError'));
  }

  // Share skills into ~/.agents/skills (additive junctions) so OpenCode + Codex see them all.
  onShareSkills() {
    this.askConfirm({
      title: t('environments.shareConfirmTitle'),
      message: t('environments.shareConfirmMsg'),
      confirmLabel: t('environments.shareConfirmBtn'),
      action: async () => {
        try {
          const r = await shareSkills();
          pushToast({
            kind: r.failed ? 'error' : 'success',
            title: t('environments.shareDone', { created: r.created, skipped: r.skipped, failed: r.failed }),
            detail: r.failed && r.details.length ? r.details.join('\n') : undefined
          });
          await this.load();
          if (this.matrix !== null) await this.loadMatrix();
        } catch (e) {
          pushToast({ kind: 'error', title: t('environments.shareError'), detail: String(e) });
        }
      }
    });
  }

  // Wrap your own slash-commands as SKILL.md into ~/.agents/skills so Codex/OpenCode can run them.
  onShareCommands() {
    this.askConfirm({
      title: t('environments.shareCmdConfirmTitle'),
      message: t('environments.shareCmdConfirmMsg'),
      confirmLabel: t('environments.shareCmdConfirmBtn'),
      action: async () => {
        try {
          const r = await shareCommands();
          pushToast({
            kind: r.failed ? 'error' : 'success',
            title: t('environments.shareCmdDone', { created: r.created, skipped: r.skipped, failed: r.failed }),
            detail: r.failed && r.details.length ? r.details.join('\n') : undefined
          });
          await this.load();
          if (this.matrix !== null) await this.loadMatrix();
        } catch (e) {
          pushToast({ kind: 'error', title: t('environments.shareError'), detail: String(e) });
        }
      }
    });
  }
}

export const envState = new EnvState();
