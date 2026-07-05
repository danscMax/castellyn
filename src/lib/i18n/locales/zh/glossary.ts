export default {
  all: '编排器：依次运行下面所有维护步骤（插件、分叉、CLI 工具、清理）。点击一次「应用」即可一次性更新整个 Claude Code 技术栈。',
  plugins:
    '来自市场的 Claude Code 插件（命令、技能和代理的集合）。「检查」查看是否有新版本；「应用」更新它们。',
  forks:
    '你在 GitHub 上的分叉及其与原仓库（upstream）的同步情况：PR 状态、已合并和开放的分支、冲突。这里仅显示状态；具体操作在「分叉」标签页。',
  rtk: 'RTK（Rust Token Killer）——一个本地 CLI 代理，可在日常开发命令（git、ls 等）上节省令牌。此步骤将 rtk 二进制文件更新到最新版本。',
  speckit:
    'SpecKit——一组用于规范和规划任务的斜杠命令（specify、plan、tasks、implement…）。此步骤更新这些命令。',
  opencode: 'opencode——一个用于处理代码的替代终端 AI 代理。此步骤将其更新到最新版本。',
  freellmapi:
    'FreeLLMAPI——一个本地的 OpenAI 兼容网关，连接免费的 LLM 提供商（在众多服务之上提供单一地址）。此步骤更新其代码。',
  cargo:
    'Cargo 二进制文件——通过「cargo install」安装的实用工具（Rust）。此步骤检查并更新它们（cargo install-update -a）。',
  bomfix:
    '配置 BOM 修复：修复意外带有 BOM 标记保存的 Claude JSON 文件——文件开头的不可见字节会导致 Claude Code 无法读取设置。它会移除该标记而不更改内容。',
  ccrrouter:
    'Claude Code Router（ccr）——一座桥梁，Claude Code 通过它与纯 OpenAI 引擎（FreeLLMAPI、DeepSeek、Qwen）通信。LM Studio 不需要桥梁——它提供原生 Anthropic 端点并直接挂载到配置上。此步骤更新 ccr（npm）；安装/配置在「提供商」标签页。在同一会话中混用提供商：ccr 已将后台/子代理请求路由到「background」路由（默认 FreeLLMAPI），你还可以在子代理提示词开头加上标签「<CCR-SUBAGENT-MODEL>提供商,模型</CCR-SUBAGENT-MODEL>」把它引导到另一个提供商。'
};
