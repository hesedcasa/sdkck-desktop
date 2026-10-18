import {useCallback, useEffect, useState} from 'react'

import type {AppContext} from '../App.tsx'
import type {PluginInfo, RegistryPackage} from '../lib/types.ts'

import {RunConsole} from '../components/RunConsole.tsx'
import {api, errorMessage} from '../lib/api.ts'
import {useRunFinished} from '../lib/runs.tsx'

/** Friendly blurbs for sdkck's first-party plugins (JIT pins carry no metadata until installed). */
export const KNOWN_PLUGINS: Record<string, string> = {
  '@hesed/bb': 'Bitbucket Cloud — repositories, pull requests, pipelines',
  '@hesed/conni': 'Confluence Cloud — spaces and pages',
  '@hesed/jira': 'Jira Cloud — issues, projects, boards',
  '@hesed/mcp-client': 'Connect external MCP servers; their tools become commands',
  '@hesed/mcp-server': 'Expose every sdkck command to MCP clients (Claude Code, Cursor…)',
  '@hesed/mysql': 'MySQL — queries and schema inspection',
  '@hesed/psql': 'PostgreSQL — queries and schema inspection',
  '@hesed/sentry': 'Sentry — issues and events',
  '@hesed/supabase': 'Supabase — projects and databases',
  '@hesed/trello': 'Trello — boards, lists and cards',
  '@hesed/webui': 'Browser UI for sdkck commands',
}

const KIND_LABEL: Record<PluginInfo['kind'], string> = {core: 'bundled', jit: 'available', link: 'linked', root: 'sdkck', user: 'installed'}

export function PluginsPage({ctx}: {ctx: AppContext}) {
  const [runId, setRunId] = useState<null | number>(null)
  const [busy, setBusy] = useState<null | string>(null)
  const [error, setError] = useState<null | string>(null)
  const [spec, setSpec] = useState('')
  const [query, setQuery] = useState('@hesed')
  const [registry, setRegistry] = useState<null | RegistryPackage[]>(null)
  const [searching, setSearching] = useState(false)

  const plugins = ctx.catalog?.plugins ?? []
  const installed = plugins.filter((p) => p.installed)
  const available = plugins.filter((p) => !p.installed)
  const installedNames = new Set(installed.map((p) => p.name))

  useRunFinished(
    runId,
    useCallback(() => {
      setBusy(null)
      ctx.refresh()
    }, [ctx]),
  )

  const start = async (label: string, action: () => Promise<number>) => {
    try {
      setError(null)
      setBusy(label)
      setRunId(await action())
    } catch (error_) {
      setBusy(null)
      setError(errorMessage(error_))
    }
  }

  const searchRegistry = useCallback(async (text: string) => {
    setSearching(true)
    try {
      setRegistry(await api.searchRegistry(text))
    } catch (error_) {
      setError(errorMessage(error_))
    } finally {
      setSearching(false)
    }
  }, [])

  useEffect(() => {
    searchRegistry('@hesed')
  }, [searchRegistry])

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>Plugins</h1>
          <p className="muted">
            Plugins are regular sdkck (oclif) plugins — installed here exactly as <code>sdkck plugins install</code> would, and shared with the
            CLI.
          </p>
        </div>
        <button className="ghost" disabled={!!busy} onClick={() => start('update', api.updatePlugins)} type="button">
          Update all
        </button>
      </header>

      {error && <div className="banner error">{error}</div>}
      {busy && <div className="banner info">Working: {busy}…</div>}
      <RunConsole compact runId={runId} />

      <h3>Installed</h3>
      <div className="cards">
        {installed.map((p) => (
          <article className="card" key={p.name}>
            <header>
              <b>{p.name}</b>
              <span className={`badge kind-${p.kind}`}>{KIND_LABEL[p.kind]}</span>
            </header>
            <p className="muted">{p.description ?? KNOWN_PLUGINS[p.name] ?? ''}</p>
            <footer>
              <span className="muted">
                v{p.version ?? '?'} · {p.commandCount} commands
              </span>
              {(p.kind === 'user' || p.kind === 'link') && (
                <button className="ghost small danger" disabled={!!busy} onClick={() => start(`uninstall ${p.name}`, () => api.uninstallPlugin(p.name))} type="button">
                  Uninstall
                </button>
              )}
            </footer>
          </article>
        ))}
      </div>

      {available.length > 0 && (
        <>
          <h3>Ready to install</h3>
          <p className="muted">sdkck installs these automatically the first time you run one of their commands — or install them now.</p>
          <div className="cards">
            {available.map((p) => (
              <article className="card" key={p.name}>
                <header>
                  <b>{p.name}</b>
                  <span className="badge kind-jit">{p.tag}</span>
                </header>
                <p className="muted">{KNOWN_PLUGINS[p.name] ?? ''}</p>
                <footer>
                  <span className="muted">{p.commandCount} commands</span>
                  <button className="primary small" disabled={!!busy} onClick={() => start(`install ${p.name}`, () => api.installPlugin(p.name))} type="button">
                    Install
                  </button>
                </footer>
              </article>
            ))}
          </div>
        </>
      )}

      <h3>Find more on npm</h3>
      <form
        className="inline"
        onSubmit={(e) => {
          e.preventDefault()
          searchRegistry(query)
        }}
      >
        <input onChange={(e) => setQuery(e.target.value)} placeholder="Search npm, e.g. @hesed or oclif-plugin" value={query} />
        <button className="ghost" disabled={searching} type="submit">
          {searching ? 'Searching…' : 'Search'}
        </button>
      </form>
      {registry && (
        <table className="table">
          <tbody>
            {registry.map((r) => (
              <tr key={r.name}>
                <td>
                  <b>{r.name}</b> <span className="muted">{r.version}</span>
                  <div className="muted">{r.description}</div>
                </td>
                <td className="right">
                  {installedNames.has(r.name) ? (
                    <span className="badge kind-user">installed</span>
                  ) : (
                    <button className="ghost small" disabled={!!busy} onClick={() => start(`install ${r.name}`, () => api.installPlugin(r.name))} type="button">
                      Install
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <h3>Install by name</h3>
      <form
        className="inline"
        onSubmit={(e) => {
          e.preventDefault()
          if (spec.trim()) start(`install ${spec}`, () => api.installPlugin(spec.trim()))
        }}
      >
        <input onChange={(e) => setSpec(e.target.value)} placeholder="@scope/plugin, plugin@1.2.3, or a git URL" spellCheck={false} value={spec} />
        <button className="primary" disabled={!!busy || !spec.trim()} type="submit">
          Install
        </button>
      </form>
    </div>
  )
}
