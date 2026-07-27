// "Couldn't check" — the reason a read did not happen, instead of a silent blank.
// An empty list and "nothing to look with" are different things; they used to look identical.
export default {
  ghMissing: "GitHub CLI isn't installed — the repository list can't be read without it.",
  ghFailed: "GitHub CLI couldn't answer. Most often this means you're not signed in.",
  ghTimeout: "GitHub CLI didn't answer within 30 seconds. This usually means network trouble.",
  ghSpawn: 'GitHub CLI could not be started.',
  ghBadOutput: "GitHub CLI returned something that couldn't be read.",
  gatewayMissing: "The local gateway isn't installed — there are no usage figures to read.",
  nodeMissing: "Node.js isn't installed — the usage figures can't be calculated without it.",
  nodeSpawn: 'Node.js could not be started.',
  analyticsQueryFailed: "The usage database couldn't be read.",
  analyticsBadOutput: "The usage helper returned something that couldn't be read.",
  howTo: 'How to install',
  showDetail: 'Details',
  hideDetail: 'Hide details'
};
