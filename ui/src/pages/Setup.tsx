import {listen} from '@tauri-apps/api/event'
import {openUrl} from '@tauri-apps/plugin-opener'
import {useEffect, useRef, useState} from 'react'

import type {CatalogSummary, RunEvent, Status} from '../lib/types.ts'

import {api, errorMessage} from '../lib/api.ts'
import {KNOWN_PLUGINS} from './Plugins.tsx'

/** Installs sdkck into the app's own runtime folder, streaming npm's log. */
export function SdkckInstaller({label, onDone}: {label: string; onDone: (s: Status) => void}) {
  const [log, setLog] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<null | string>(null)
  const ref = useRef<HTMLPreElement>(null)

  useEffect(() => {
    const unlisten = listen<string>('setup-log', (e) => setLog((l) => [...l.slice(-400), e.payload]))
    return () => {
      unlisten.then((f) => f())
    }
  }, [])
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight
  }, [log])

  const install = async () => {
    setBusy(true)
    setError(null)
    setLog([])
    try {
      onDone(await api.installSdkck())
    } catch (error_) {
      setError(errorMessage(error_))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="installer">
      <button className="primary" disabled={busy} onClick={install} type="button">
        {busy ? 'Installing…' : label}
      </button>
      {error && <div className="banner error">{error}</div>}
      {log.length > 0 && (
        <pre className="log" ref={ref}>
          {log.join('\n')}
        </pre>
      )}
    </div>
  )
}

function waitForRun(runId: number): Promise<number> {
  return new Promise((resolve) => {
    const off = listen<RunEvent>('run-event', (e) => {
      if (e.payload.runId === runId && e.payload.type === 'finished') {
        off.then((f) => f())
        resolve(e.payload.code)
      }
    })
  })
}

type Props = {onDone: () => Promise<void>; setStatus: (s: Status) => void; status: Status}

export function SetupWizard({onDone, setStatus, status}: Props) {
  const nodeOk = status.node?.supported ?? false
  const sdkckOk = Boolean(status.sdkck)
  const [catalog, setCatalog] = useState<CatalogSummary | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set(['@hesed/jira', '@hesed/mcp-client']))
  const [progress, setProgress] = useState<Record<string, 'done' | 'failed' | 'installing'>>({})
  const [installing, setInstalling] = useState(false)
  const [nodePath, setNodePath] = useState('')
  const [error, setError] = useState<null | string>(null)

  useEffect(() => {
    if (nodeOk && sdkckOk) api.catalog().then(setCatalog, (e) => setError(errorMessage(e)))
  }, [nodeOk, sdkckOk, status.sdkck?.root])

  const step = !nodeOk ? 1 : !sdkckOk ? 2 : 3
  const available = catalog?.plugins.filter((p) => !p.installed) ?? []
  const installed = catalog?.plugins.filter((p) => p.kind === 'user' || p.kind === 'link') ?? []

  const toInstall = available.map((p) => p.name).filter((n) => picked.has(n))
  const installPicked = async () => {
    setInstalling(true)
    for (const name of toInstall) {
      setProgress((p) => ({...p, [name]: 'installing'}))
      try {
        const runId = await api.installPlugin(name)
        const code = await waitForRun(runId)
        setProgress((p) => ({...p, [name]: code === 0 ? 'done' : 'failed'}))
      } catch {
        setProgress((p) => ({...p, [name]: 'failed'}))
      }
    }

    setInstalling(false)
    setCatalog(await api.catalog())
  }

  const finish = async () => {
    try {
      setStatus(await api.saveSettings({...status.settings, onboarded: true}))
      await onDone()
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }

  return (
    <div className="wizard">
      <div className="wizard-card">
        <div className="logo">sdkck</div>
        <h1>Welcome to sdkck Desktop</h1>
        <p className="muted">Three quick steps and you’re ready to run sdkck commands from here — no terminal needed.</p>
        <ol className="steps">
          <li className={step > 1 ? 'done' : 'current'}>Node.js</li>
          <li className={step > 2 ? 'done' : step === 2 ? 'current' : ''}>sdkck</li>
          <li className={step === 3 ? 'current' : ''}>Plugins</li>
        </ol>
        {error && <div className="banner error">{error}</div>}

        {step === 1 && (
          <section>
            <h2>Install Node.js {status.node ? 'update' : ''}</h2>
            {status.node ? (
              <p>
                Found Node.js {status.node.version} at <code>{status.node.path}</code>, but sdkck needs version 22 or newer.
              </p>
            ) : (
              <p>sdkck’s plugins run on Node.js 22 or newer, which wasn’t found on this computer.</p>
            )}
            <div className="row">
              <button className="primary" onClick={() => openUrl('https://nodejs.org/en/download')} type="button">
                Download Node.js
              </button>
              <button className="ghost" onClick={() => api.rediscover().then(setStatus)} type="button">
                I’ve installed it — check again
              </button>
            </div>
            <details>
              <summary>Node is installed somewhere unusual</summary>
              <form
                className="inline"
                onSubmit={async (e) => {
                  e.preventDefault()
                  setStatus(await api.saveSettings({...status.settings, nodePath: nodePath || null}))
                }}
              >
                <input onChange={(e) => setNodePath(e.target.value)} placeholder="/path/to/node" spellCheck={false} value={nodePath} />
                <button className="ghost" type="submit">
                  Use this
                </button>
              </form>
            </details>
          </section>
        )}

        {step === 2 && (
          <section>
            <h2>Install sdkck</h2>
            <p>
              Node.js {status.node?.version} is ready. sdkck Desktop will install the sdkck CLI into its own folder (
              <code>{status.managedPrefix}</code>) — your global npm setup is left untouched. Already have sdkck installed globally? It is
              picked up automatically; use “check again” after installing it.
            </p>
            <div className="row">
              <SdkckInstaller label="Install sdkck" onDone={setStatus} />
              <button className="ghost" onClick={() => api.rediscover().then(setStatus)} type="button">
                Check again
              </button>
            </div>
          </section>
        )}

        {step === 3 && (
          <section>
            <h2>Choose your plugins</h2>
            <p className="muted">
              sdkck {status.sdkck?.version} is ready ({status.sdkck?.origin}). Pick the tools you use; you can add or remove plugins any time
              from the Plugins page.
            </p>
            {installed.length > 0 && <p>Already installed: {installed.map((p) => p.name).join(', ')}</p>}
            <div className="picker">
              {available.map((p) => (
                <label className="pick" key={p.name}>
                  <input
                    checked={picked.has(p.name)}
                    disabled={installing || progress[p.name] === 'done'}
                    onChange={(e) => {
                      const next = new Set(picked)
                      if (e.target.checked) next.add(p.name)
                      else next.delete(p.name)
                      setPicked(next)
                    }}
                    type="checkbox"
                  />
                  <div>
                    <b>{p.name.replace('@hesed/', '')}</b>
                    <small>{KNOWN_PLUGINS[p.name] ?? p.name}</small>
                  </div>
                  {progress[p.name] && <span className={`pill ${progress[p.name]}`}>{progress[p.name]}</span>}
                </label>
              ))}
              {catalog && available.length === 0 && <p className="muted">Every available plugin is already installed.</p>}
            </div>
            <div className="row">
              <button className="ghost" disabled={installing || toInstall.length === 0} onClick={installPicked} type="button">
                {installing ? 'Installing…' : 'Install selected'}
              </button>
              <button className="primary" disabled={installing} onClick={finish} type="button">
                {Object.keys(progress).length > 0 || available.length === 0 ? 'Start using sdkck' : 'Skip for now'}
              </button>
            </div>
          </section>
        )}
      </div>
    </div>
  )
}
