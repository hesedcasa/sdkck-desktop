// Mirrors of the serde types in crates/sdkck-core (camelCase on the wire).

export type FlagMeta = {
  allowNo: boolean
  char: null | string
  default: unknown
  description: null | string
  kind: 'boolean' | 'option'
  multiple: boolean
  name: string
  options: null | string[]
  required: boolean
}

export type ArgMeta = {
  default: unknown
  description: null | string
  name: string
  options: null | string[]
  required: boolean
}

export type Decision = {allowed: boolean; pattern?: string; reason: 'allow-rule' | 'disallow-rule' | 'no-allow-rule' | 'permission-command'}

export type CommandMeta = {
  aliases: string[]
  args: ArgMeta[]
  description: null | string
  displayId: string
  examples: string[]
  flags: FlagMeta[]
  id: string
  permission: Decision
  pluginName: string
  pluginType: string
  source: 'api' | 'jit' | 'mcp' | 'plugin'
  summary: null | string
  topic: string
}

export type PluginInfo = {
  commandCount: number
  description: null | string
  installed: boolean
  kind: 'core' | 'jit' | 'link' | 'root' | 'user'
  name: string
  root: null | string
  tag: null | string
  version: null | string
}

export type Topic = {description: null | string; id: string}

export type CatalogSummary = {
  bin: string
  blockedCount: number
  buildMicros: number
  commandCount: number
  plugins: PluginInfo[]
  topics: Topic[]
  version: null | string
  warnings: string[]
}

export type SearchResult = {commands: CommandMeta[]; searchMicros: number; total: number}

export type Settings = {
  env: Record<string, string>
  historyLimit: number
  loginShellEnv: boolean
  nodePath: null | string
  onboarded: boolean
  sdkckRoot: null | string
  warmRunner: boolean
}

export type Interception =
  | {message: string; state: 'misconfigured'}
  | {proxy: string; state: 'platform'}
  | {state: 'broker'; vault: string}
  | {state: 'off'}

export type WorkerState = 'booting' | 'busy' | 'failed' | 'off' | 'ready'

export type Status = {
  dirs: {cache: string; config: string; data: string}
  interception: Interception
  managedPrefix: string
  node: null | {major: number; path: string; supported: boolean; version: string}
  problems: string[]
  ready: boolean
  sdkck: null | {origin: 'custom' | 'global' | 'managed'; root: string; version: null | string}
  settings: Settings
  worker: WorkerState
}

export type RunMode = 'cold' | 'warm'

export type RunEvent =
  | {argv: string[]; mode: RunMode; runId: number; type: 'started'}
  | {cancelled: boolean; code: number; durationMs: number; mode: RunMode; runId: number; type: 'finished'}
  | {data: string; runId: number; stream: 'stderr' | 'stdout'; type: 'output'}

export type HistoryEntry = {
  argv: string[]
  cancelled: boolean
  code: null | number
  commandId: null | string
  durationMs: null | number
  mode: null | RunMode
  output: string
  runId: number
  startedAt: number
  truncated: boolean
}

export type Rule = {pattern: string}
export type PermissionView = {
  config: {allowRules: Rule[]; denyRules: Rule[]}
  error: null | string
  exists: boolean
  path: string
}

export type RegistryPackage = {
  date: null | string
  description: null | string
  homepage: null | string
  keywords: string[]
  name: string
  version: string
}
