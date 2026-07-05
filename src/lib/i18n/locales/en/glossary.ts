export default {
  all: 'Orchestrator: runs all the maintenance steps below one after another (plugins, forks, CLI tools, cleanup). A single click on “Apply” updates the entire Claude Code stack at once.',
  plugins:
    'Claude Code plugins from marketplaces (bundles of commands, skills and agents). “Check” looks for newer versions; “Apply” updates them.',
  forks:
    'Your GitHub forks and how they sync with their originals (upstream): PR status, merged and open branches, conflicts. Here it is status only; the actions themselves live on the “Forks” tab.',
  rtk: 'RTK (Rust Token Killer) — a local CLI proxy that saves tokens on routine dev commands (git, ls, etc.). This step updates the rtk binary to the latest version.',
  speckit:
    'SpecKit — a set of slash commands for specifying and planning tasks (specify, plan, tasks, implement…). The step updates these commands.',
  opencode: 'opencode — an alternative terminal AI agent for working with code. The step updates it to the latest version.',
  freellmapi:
    'FreeLLMAPI — a local OpenAI-compatible gateway to free LLM providers (one address on top of many). The step updates its code.',
  cargo:
    'Cargo binaries — utilities installed via “cargo install” (Rust). The step checks and updates them (cargo install-update -a).',
  bomfix:
    'Config BOM-fix: repairs Claude JSON files accidentally saved with a BOM mark — invisible bytes at the start of the file that can keep Claude Code from reading the settings. It removes the mark without changing the contents.',
  ccrrouter:
    'Claude Code Router (ccr) — a bridge through which Claude Code talks to pure OpenAI engines (FreeLLMAPI, DeepSeek, Qwen). LM Studio needs no bridge — it serves a native Anthropic endpoint and attaches to the profile directly. The step updates ccr (npm); installation/setup is on the “Providers” tab. Mixing providers in one session: ccr already routes background/subagent traffic to the “background” route (FreeLLMAPI by default), and you can send a specific subagent to another provider by prefixing its prompt with the tag “<CCR-SUBAGENT-MODEL>provider,model</CCR-SUBAGENT-MODEL>”.'
};
