<script lang="ts">
  // Subagents manager: view/create/edit/delete standalone user subagents in ~/.claude/agents/*.md.
  // Plugin-bundled agents (read-only) live in the Plugins tab; this tab owns only editable user
  // agents. A save fans out to every profile + machine via the existing `agents` folder sync — no
  // extra step here (see ipc.ts saveAgent / lib.rs agents_dir comment).
  import type { AgentInfo } from '$lib/ipc';
  import { readAgent } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import EmptyState from './EmptyState.svelte';
  import ModalShell from './ModalShell.svelte';
  import Select from './Select.svelte';
  import { Bot, Pencil, Trash2, Plus } from '@lucide/svelte';

  let {
    data,
    running,
    onSave,
    onDelete,
    onRefresh,
    onOpenExtensions
  }: {
    data: AgentInfo[] | null;
    running: string | null;
    // Resolves once the write completed (so the dialog only closes on success).
    onSave: (a: {
      name: string;
      description: string;
      model: string;
      tools: string;
      prompt: string;
      path?: string;
    }) => Promise<void>;
    onDelete: (a: AgentInfo) => void;
    onRefresh: () => void;
    onOpenExtensions: () => void;
  } = $props();

  const busy = $derived(!!running);
  const agents = $derived(data ?? []);

  // Codex-backed template body: calls the `codex` CLI DIRECTLY. It deliberately does NOT copy the
  // installed codex-rescue subagent, which forwards to a plugin-internal script ($CLAUDE_PLUGIN_ROOT/
  // codex-companion.mjs) + plugin skills — neither exists for a standalone ~/.claude/agents file.
  const CODEX_BODY = `You are a thin wrapper that forwards the user's task to the Codex CLI.

Use exactly one Bash call:

    codex exec "<the user's task>"

Rules:
- Forward the task text as-is; do not solve it yourself.
- Do not read files, grep, or do independent work beyond running the command.
- Return the command's stdout exactly as-is, with no commentary.`;

  // --- Editor state ---
  let open = $state(false);
  let origPath = $state<string | undefined>(undefined); // set → editing an existing file
  let tpl = $state(''); // transient template picker (create only)
  let name = $state('');
  let description = $state('');
  let model = $state('');
  let tools = $state('');
  let prompt = $state('');
  let loading = $state(false); // full file (prompt) is being read
  let saving = $state(false);

  const editing = $derived(!!origPath);

  const MODEL_PRESETS = ['sonnet', 'opus', 'haiku'];
  const modelOptions = $derived([
    { value: '', label: t('agents.modelInherit') },
    ...MODEL_PRESETS.map((m) => ({ value: m, label: m })),
    // Keep a custom model id loaded from an existing file selectable.
    ...(model && !MODEL_PRESETS.includes(model) ? [{ value: model, label: model }] : [])
  ]);

  function resetForm() {
    origPath = undefined;
    tpl = '';
    name = '';
    description = '';
    model = '';
    tools = '';
    prompt = '';
  }

  function openCreate() {
    resetForm();
    open = true;
  }

  // Prefill the create form from a template (English seed content — subagent files are English by
  // ecosystem convention; the user edits from here).
  function applyTemplate(v: string) {
    tpl = v;
    if (v === 'codex') {
      name = 'codex-delegate';
      description =
        'Delegate a substantial coding or debugging task to the Codex CLI for a second implementation or diagnosis pass.';
      model = 'sonnet';
      tools = 'Bash';
      prompt = CODEX_BODY;
    } else {
      name = '';
      description = '';
      model = '';
      tools = '';
      prompt = '';
    }
  }

  async function openEdit(a: AgentInfo) {
    resetForm();
    origPath = a.path;
    // Show the light list row immediately while the full file (prompt body) loads.
    name = a.name;
    description = a.description;
    model = a.model;
    tools = a.tools;
    open = true;
    loading = true;
    try {
      const d = await readAgent(a.path);
      name = d.name;
      description = d.description;
      model = d.model;
      tools = d.tools;
      prompt = d.prompt;
    } catch {
      /* keep the row values; body stays empty */
    } finally {
      loading = false;
    }
  }

  const canSave = $derived(!!name.trim() && !saving && !loading);

  async function save() {
    if (!canSave) return;
    saving = true;
    try {
      await onSave({
        name: name.trim(),
        description: description.trim(),
        model,
        tools: tools.trim(),
        prompt,
        path: origPath
      });
      open = false;
    } catch {
      /* onSave surfaces its own error toast; keep the dialog open so nothing is lost */
    } finally {
      saving = false;
    }
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('agents.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('agents.subtitle')}</p>
    </div>
    <div class="flex shrink-0 items-center gap-sw-2">
      <button class="sw-btn sw-btn-ghost" disabled={busy} onclick={onRefresh} title={t('agents.refreshTitle')}>
        {running === 'agents' ? t('common.busy') : t('common.refresh')}
      </button>
      <button class="sw-btn sw-btn-primary" onclick={openCreate} title={t('agents.createTip')}>
        <Plus size={14} aria-hidden="true" />
        {t('agents.create')}
      </button>
    </div>
  </header>

  {#if data === null}
    <div class="grid gap-sw-3 md:grid-cols-2">
      {#each Array(4) as _, i (i)}<div class="skeleton" style="height:6rem;width:100%"></div>{/each}
    </div>
  {:else if agents.length === 0}
    <EmptyState
      icon={Bot}
      title={t('agents.emptyTitle')}
      description={t('agents.emptyDesc')}
      action={openCreate}
      actionLabel={t('agents.emptyAction')}
    />
    <p class="mt-sw-2 text-center text-sw-xs text-sw-text-muted">
      {t('agents.pluginNote')}
      <button class="underline hover:text-sw-text" onclick={onOpenExtensions}>{t('agents.pluginNoteLink')}</button>
    </p>
  {:else}
    <div class="grid gap-sw-3 md:grid-cols-2">
      {#each agents as a (a.path)}
        <div class="sw-card flex flex-col gap-sw-2">
          <div class="flex items-center justify-between gap-sw-2">
            <span class="text-base font-semibold truncate" title={a.name}>{a.name}</span>
            <div class="flex shrink-0 items-center gap-sw-1">
              <button class="sw-btn sw-btn-ghost text-sw-sm" onclick={() => openEdit(a)}
                title={t('agents.edit')} aria-label={t('agents.edit')}><Pencil size={14} aria-hidden="true" /></button>
              <button class="sw-btn sw-btn-ghost text-sw-sm" onclick={() => onDelete(a)}
                title={t('agents.delete')} aria-label={t('agents.delete')}><Trash2 size={14} aria-hidden="true" /></button>
            </div>
          </div>
          {#if a.description}<p class="text-sw-sm text-sw-text-secondary">{a.description}</p>{/if}
          <div class="flex flex-wrap items-center gap-sw-1 text-sw-xs">
            <span class="badge badge-info">{a.model || t('agents.modelInherit')}</span>
            <span class="badge badge-muted" title={a.tools || t('agents.toolsAll')}>{a.tools || t('agents.toolsAll')}</span>
          </div>
        </div>
      {/each}
    </div>
    <p class="mt-sw-4 text-sw-xs text-sw-text-muted">
      {t('agents.pluginNote')}
      <button class="underline hover:text-sw-text" onclick={onOpenExtensions}>{t('agents.pluginNoteLink')}</button>
    </p>
  {/if}
</div>

<ModalShell {open} onClose={() => (open = false)} onEnter={save} size="lg">
  <h3 class="dlg-h">{editing ? t('agents.editTitle') : t('agents.createTitle')}</h3>

  {#if !editing}
    <div class="dlg-fld">
      <span>{t('agents.template')}</span>
      <Select
        value={tpl}
        onChange={applyTemplate}
        options={[
          { value: '', label: t('agents.tplBlank') },
          { value: 'codex', label: t('agents.tplCodex') }
        ]}
      />
    </div>
  {/if}

  <label class="dlg-fld">
    <span>{t('agents.fldName')}</span>
    <input class="sw-input" bind:value={name} placeholder={t('agents.fldNamePh')} spellcheck="false" />
  </label>

  <label class="dlg-fld">
    <span>{t('agents.fldDesc')}</span>
    <input class="sw-input" bind:value={description} placeholder={t('agents.fldDescPh')} />
  </label>

  <div class="grid grid-cols-2 gap-sw-3">
    <div class="dlg-fld">
      <span>{t('agents.fldModel')}</span>
      <Select bind:value={model} options={modelOptions} />
    </div>
    <label class="dlg-fld">
      <span>{t('agents.fldTools')}</span>
      <input class="sw-input" bind:value={tools} placeholder={t('agents.fldToolsPh')} spellcheck="false" />
    </label>
  </div>

  <label class="dlg-fld">
    <span>{t('agents.fldPrompt')}</span>
    <textarea class="sw-input" rows="10" bind:value={prompt}
      placeholder={loading ? t('common.busy') : t('agents.fldPromptPh')} spellcheck="false"></textarea>
  </label>

  <div class="dlg-row">
    <button class="sw-btn sw-btn-ghost" onclick={() => (open = false)}>{t('common.cancel')}</button>
    <button class="sw-btn sw-btn-primary" disabled={!canSave} onclick={save} title={t('agents.saveTip')}>
      {saving ? t('common.busy') : t('agents.save')}
    </button>
  </div>
</ModalShell>
