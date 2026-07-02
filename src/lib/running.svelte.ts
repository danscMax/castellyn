// App-wide "what's running now" indicator. The actual run lock lives in +page.svelte (`running`);
// this store just mirrors it so the title bar (a sibling of the page in +layout) can show it
// without prop-drilling across the layout boundary. One $effect in +page keeps it in sync.
import { t } from '$lib/i18n';

export const runningStore = $state<{ op: string | null }>({ op: null });

// Friendly names for operational (non-component) runs. Component runs pass their id through as-is.
const OP_KEYS: Record<string, string> = {
  backup: 'op_backup',
  profiles: 'op_profiles',
  mcp: 'op_mcp',
  sync: 'op_sync',
  engine: 'op_engine',
  provider: 'op_provider',
  schedule: 'op_schedule',
  forks: 'op_forks',
  'plugin-mgr': 'op_plugins',
  pluginsync: 'op_pluginsync'
};
export const opName = (id: string) => (OP_KEYS[id] ? t(`page.${OP_KEYS[id]}`) : id);
