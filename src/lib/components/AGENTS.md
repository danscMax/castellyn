# src/lib/components — Svelte components

## Purpose

The visual layer: one component per maintenance/feature tab, plus dialogs and shell chrome.
Components render state and raise events; orchestration lives in `routes/+page.svelte`.

## Ownership

- Tabs (one per feature): `HomeTab`, `UpdatesTab`, `PluginsTab`, `ForksTab`, `McpTab`,
  `ProvidersTab`, `ProfilesTab`, `SessionsTab`, `EnvironmentsTab`, `SyncTab`, `ScheduleTab`,
  `BackupTab`, `AnalyticsTab`, `SettingsTab`
- Shell / chrome: `Sidebar`, `WindowTitleBar`, `Console`, `ToastHost`, `NotificationPanel`,
  `CommandPalette`, `HotkeyHelp`, `SectionHeader`, `EmptyState`, `Spinner`
- Dialogs / modals: `ConfirmDialog`, `ModalShell`, `ProfileEditDialog`, `ProviderEditDialog`,
  `MyProviderEditDialog`, `RestoreDialog`, `RouterConnectDialog`, `LaunchConfigDialog`,
  `OnboardingWizard`
- Reusable primitives: `Toggle`, `Select`, `DropdownMenu`, `DataTable`, `Sparkline`,
  `SecretInput`, `FolderField`, `ComponentCard`, `ProfileUsageBadge`
- Sessions/agents: `SessionsTab`, `TerminalPane`, `DetachedView`
- Profiles/stack cards: `ProfilesMatrix`, `StackHealthCard`, `StackDriftCard`, `StackGcCard`,
  `ForkRepoCard`

## Local Contracts

- Destructive actions gate behind `ConfirmDialog` via `askConfirm`; never trigger a native dialog
- All strings via `t('ns.key')`; each tab maps to its locale namespace under `i18n/locales/*`
- Presentational: take props / emit events; do not call `invoke` directly — go through the
  orchestrator or `lib/ipc.ts`
- Reuse primitives (`Toggle`, `Select`, `DataTable`, `ModalShell`) instead of re-styling ad hoc

## Work Guidance

- New tab → add its i18n namespace, register in `navOrder.svelte.ts`, wire runs in
  `routes/+page.svelte`
- UI is not "done" until rendered and visually checked in both themes (see `CLAUDE.md`)

## Verification

- `npm run check` (0/0), `npm test`

## Child DOX Index

None.
