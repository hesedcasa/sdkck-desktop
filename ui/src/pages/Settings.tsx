import {openPath} from '@tauri-apps/plugin-opener'
import {useState} from 'react'

import type {AppContext} from '../App.tsx'
import type {Settings, Status} from '../lib/types.ts'

import {api, errorMessage} from '../lib/api.ts'
import {SdkckInstaller} from './Setup.tsx'

type Props = {ctx: AppContext; setStatus: (s: Status) => void}

export function SettingsPage({ctx, setStatus}: Props) {
  const {status} = ctx
  const [draft, setDraft] = useState<Settings>(status.settings)
  const [envRows, setEnvRows] = useState<[string, string][]>(Object.entries(status.settings.env))
  const [reveal, setReveal] = useState(false)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<null | string>(null)

  const save = async (next: Settings = draft) => {
    try {
      setError(null)
      const env = Object.fromEntries(envRows.filter(([k]) => k.trim()).map(([k, v]) => [k.trim(), v]))
      const s = await api.saveSettings({...next, env})
      setStatus(s)
      setDraft(s.settings)
      await ctx.refresh()
      setSaved(true)
      setTimeout(() => setSaved(false), 1500)
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }

  const folder = (label: string, path: string) => (
    <div className="kv">
      <span>{label}</span>
      <code>{path}</code>
      <button className="ghost small" onClick={() => openPath(path).catch((e) => setError(errorMessage(e)))} type="button">
        Open
      </button>
    </div>
  )

  return (
    <div className="page settings">
      <header className="page-head">
        <h1>Settings</h1>
        <div className="row">
          {saved && <span className="good-text">Saved</span>}
          <button className="primary" onClick={() => save()} type="button">
            Save
          </button>
        </div>
      </header>
      {error && <div className="banner error">{error}</div>}

      <section>
        <h3>Performance</h3>
        <label className="field check">
          <input checked={draft.warmRunner} onChange={(e) => setDraft({...draft, warmRunner: e.target.checked})} type="checkbox" />
          <div>
            <span className="field-name">Warm runner</span>
            <small>
              Keep one Node process booted with sdkck loaded so commands start in milliseconds instead of re-booting Node each time. Plugin
              installs, long-running servers and Agent Vault–brokered sessions always use a fresh process. Current state: <b>{status.worker}</b>
            </small>
          </div>
        </label>
        <button className="ghost small" onClick={() => api.restartWorker().then(ctx.refresh)} type="button">
          Restart warm runner
        </button>
      </section>

      <section>
        <h3>Environment</h3>
        <label className="field check">
          <input checked={draft.loginShellEnv} onChange={(e) => setDraft({...draft, loginShellEnv: e.target.checked})} type="checkbox" />
          <div>
            <span className="field-name">Use my login shell’s environment</span>
            <small>Apps opened from the dock or start menu don’t see variables from ~/.zshrc or ~/.bashrc. Turn this on to import them.</small>
          </div>
        </label>
        <p className="muted">Variables passed to every command (API keys, OPENAI_API_KEY, SDKCK_CONFIG_DIR, AGENT_VAULT_*…):</p>
        <div className="env-table">
          {envRows.map(([k, v], i) => (
            <div className="env-row" key={i}>
              <input
                onChange={(e) => setEnvRows(envRows.map((r, j) => (j === i ? [e.target.value, r[1]] : r)))}
                placeholder="NAME"
                spellCheck={false}
                value={k}
              />
              <input
                onChange={(e) => setEnvRows(envRows.map((r, j) => (j === i ? [r[0], e.target.value] : r)))}
                placeholder="value"
                spellCheck={false}
                type={reveal ? 'text' : 'password'}
                value={v}
              />
              <button className="ghost small" onClick={() => setEnvRows(envRows.filter((_, j) => j !== i))} type="button">
                Remove
              </button>
            </div>
          ))}
          <div className="row">
            <button className="ghost small" onClick={() => setEnvRows([...envRows, ['', '']])} type="button">
              Add variable
            </button>
            <label className="row muted">
              <input checked={reveal} onChange={(e) => setReveal(e.target.checked)} type="checkbox" /> Show values
            </label>
          </div>
        </div>
      </section>

      <section>
        <h3>Runtime</h3>
        <div className="kv">
          <span>Node.js</span>
          <code>
            {status.node ? `${status.node.version} — ${status.node.path}` : 'not found'}
          </code>
        </div>
        <label className="field">
          <span className="field-name">Node.js path override</span>
          <input
            onChange={(e) => setDraft({...draft, nodePath: e.target.value || null})}
            placeholder="Detected automatically"
            spellCheck={false}
            value={draft.nodePath ?? ''}
          />
        </label>
        <div className="kv">
          <span>sdkck</span>
          <code>
            {status.sdkck ? `${status.sdkck.version} (${status.sdkck.origin}) — ${status.sdkck.root}` : 'not found'}
          </code>
        </div>
        <label className="field">
          <span className="field-name">sdkck package folder override</span>
          <input
            onChange={(e) => setDraft({...draft, sdkckRoot: e.target.value || null})}
            placeholder="App-managed install, else the global one"
            spellCheck={false}
            value={draft.sdkckRoot ?? ''}
          />
          <small>Point at a sdkck checkout (the folder with package.json and bin/run.js) to use a development build.</small>
        </label>
        <SdkckInstaller label={status.sdkck?.origin === 'managed' ? 'Update managed sdkck' : 'Install a managed copy of sdkck'} onDone={(s) => setStatus(s)} />
      </section>

      <section>
        <h3>Folders</h3>
        {folder('Config (rules, API specs, MCP servers)', status.dirs.config)}
        {folder('Data (installed plugins)', status.dirs.data)}
        {folder('Cache', status.dirs.cache)}
        {folder('Managed runtime', status.managedPrefix)}
      </section>

      <section>
        <h3>Setup</h3>
        <button className="ghost" onClick={() => save({...draft, onboarded: false})} type="button">
          Run the setup wizard again
        </button>
      </section>
    </div>
  )
}
