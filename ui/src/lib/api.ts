import {invoke} from '@tauri-apps/api/core'

import type {
  CatalogSummary,
  CommandMeta,
  Decision,
  HistoryEntry,
  PermissionView,
  RegistryPackage,
  SearchResult,
  Settings,
  Status,
} from './types.ts'

// Typed wrappers over the Tauri commands in src-tauri/src/lib.rs.
export const api = {
  addPermissionRule: (kind: 'allow' | 'deny', pattern: string) => invoke<PermissionView>('add_permission_rule', {kind, pattern}),
  blockedCommands: () => invoke<CommandMeta[]>('blocked_commands'),
  cancelRun: (runId: number) => invoke<boolean>('cancel_run', {runId}),
  catalog: () => invoke<CatalogSummary>('catalog'),
  checkPermission: (id: string) => invoke<Decision>('check_permission', {id}),
  clearHistory: () => invoke<void>('clear_history'),
  command: (id: string) => invoke<CommandMeta>('command', {id}),
  history: () => invoke<HistoryEntry[]>('history'),
  installPlugin: (spec: string) => invoke<number>('install_plugin', {spec}),
  installSdkck: (version?: string) => invoke<Status>('install_sdkck', {version}),
  permissions: () => invoke<PermissionView>('permissions'),
  rediscover: () => invoke<Status>('rediscover'),
  removePermissionRule: (kind: 'allow' | 'deny', pattern: string) =>
    invoke<PermissionView>('remove_permission_rule', {kind, pattern}),
  resetPermissions: () => invoke<PermissionView>('reset_permissions'),
  restartWorker: () => invoke<void>('restart_worker'),
  runCommand: (id: string, args: string[]) => invoke<number>('run_command', {args, id}),
  runRaw: (argv: string[]) => invoke<number>('run_raw', {argv}),
  saveSettings: (settings: Settings) => invoke<Status>('save_settings', {settings}),
  search: (query: string, limit?: number) => invoke<SearchResult>('search', {limit, query}),
  searchRegistry: (text: string) => invoke<RegistryPackage[]>('search_registry', {text}),
  status: () => invoke<Status>('status'),
  uninstallPlugin: (name: string) => invoke<number>('uninstall_plugin', {name}),
  updatePlugins: () => invoke<number>('update_plugins'),
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
