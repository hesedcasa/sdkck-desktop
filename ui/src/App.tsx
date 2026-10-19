import {useCallback, useEffect, useState} from 'react'

import type {CatalogSummary, Status} from './lib/types.ts'

import {StatusBar} from './components/StatusBar.tsx'
import {api, errorMessage} from './lib/api.ts'
import {CommandsPage} from './pages/Commands.tsx'
import {HistoryPage} from './pages/History.tsx'
import {PermissionsPage} from './pages/Permissions.tsx'
import {PluginsPage} from './pages/Plugins.tsx'
import {SettingsPage} from './pages/Settings.tsx'
import {SetupWizard} from './pages/Setup.tsx'

export type Page = 'commands' | 'history' | 'permissions' | 'plugins' | 'settings'

const NAV: {icon: string; id: Page; label: string}[] = [
  {icon: '⌘', id: 'commands', label: 'Commands'},
  {icon: '⧉', id: 'plugins', label: 'Plugins'},
  {icon: '⛨', id: 'permissions', label: 'Permissions'},
  {icon: '↺', id: 'history', label: 'History'},
  {icon: '⚙', id: 'settings', label: 'Settings'},
]

export type AppContext = {
  catalog: CatalogSummary | null
  /** Re-read status and catalog (after installs, settings changes, rule edits). */
  refresh: () => Promise<void>
  /** Jump to a command, optionally with a raw argv prefilled. */
  openCommand: (id: string) => void
  status: Status
}

export function App() {
  const [status, setStatus] = useState<null | Status>(null)
  const [catalog, setCatalog] = useState<CatalogSummary | null>(null)
  const [page, setPage] = useState<Page>('commands')
  const [selected, setSelected] = useState<null | string>(null)
  const [error, setError] = useState<null | string>(null)

  const refresh = useCallback(async () => {
    try {
      const s = await api.status()
      setStatus(s)
      setCatalog(s.ready ? await api.catalog() : null)
      setError(null)
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }, [])

  useEffect(() => {
    refresh()
    // The worker boots in the background; keep the status bar current.
    const timer = setInterval(() => api.status().then(setStatus).catch(() => {}), 3000)
    return () => clearInterval(timer)
  }, [refresh])

  // Becoming ready outside `refresh` (setup steps, clearing a bad override)
  // must still load the catalog for topics and the status bar.
  const ready = status?.ready ?? false
  const sdkckRoot = status?.sdkck?.root
  useEffect(() => {
    if (ready) api.catalog().then(setCatalog, (error_) => setError(errorMessage(error_)))
  }, [ready, sdkckRoot])

  const openCommand = useCallback((id: string) => {
    setSelected(id)
    setPage('commands')
  }, [])

  if (!status) {
    return (
      <div className="splash">
        <div className="logo">sdkck</div>
        <p className="muted">{error ?? 'Finding Node.js and sdkck…'}</p>
      </div>
    )
  }

  if (!status.ready || !status.settings.onboarded) {
    return <SetupWizard onDone={refresh} status={status} setStatus={setStatus} />
  }

  const ctx: AppContext = {catalog, openCommand, refresh, status}
  return (
    <div className="shell">
      <nav className="rail">
        <div className="brand" title="sdkck Desktop">
          sdk<span>ck</span>
        </div>
        {NAV.map((n) => (
          <button className={page === n.id ? 'nav active' : 'nav'} key={n.id} onClick={() => setPage(n.id)} type="button">
            <span aria-hidden className="nav-icon">
              {n.icon}
            </span>
            {n.label}
          </button>
        ))}
      </nav>
      <main className="content">
        {error && <div className="banner error">{error}</div>}
        {page === 'commands' && <CommandsPage ctx={ctx} selected={selected} setSelected={setSelected} />}
        {page === 'plugins' && <PluginsPage ctx={ctx} />}
        {page === 'permissions' && <PermissionsPage ctx={ctx} />}
        {page === 'history' && <HistoryPage ctx={ctx} />}
        {page === 'settings' && <SettingsPage ctx={ctx} setStatus={setStatus} />}
      </main>
      <StatusBar catalog={catalog} status={status} />
    </div>
  )
}
