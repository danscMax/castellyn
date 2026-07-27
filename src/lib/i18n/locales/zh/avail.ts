// “无法检查” — 读取未能进行的原因，而不是无声的空白。
// 空列表与“没有工具可查”是两回事，此前二者看起来完全一样。
export default {
  ghMissing: '未安装 GitHub CLI — 没有它就无法读取仓库列表。',
  ghFailed: 'GitHub CLI 无法响应。通常表示尚未登录。',
  ghTimeout: 'GitHub CLI 在 30 秒内没有响应，通常是网络问题。',
  ghSpawn: '无法启动 GitHub CLI。',
  ghBadOutput: 'GitHub CLI 返回了无法解析的内容。',
  gatewayMissing: '未安装本地网关 — 没有可读取的用量数据。',
  nodeMissing: '未安装 Node.js — 没有它就无法统计用量。',
  nodeSpawn: '无法启动 Node.js。',
  analyticsQueryFailed: '无法读取用量数据库。',
  analyticsBadOutput: '用量脚本返回了无法解析的内容。',
  howTo: '如何安装',
  showDetail: '详细信息',
  hideDetail: '隐藏详细信息'
};
