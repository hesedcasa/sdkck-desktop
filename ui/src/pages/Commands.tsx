import {useCallback, useEffect, useMemo, useRef, useState} from 'react'

import type {AppContext} from '../App.tsx'
import type {CommandMeta, SearchResult} from '../lib/types.ts'

import {CommandForm} from '../components/CommandForm.tsx'
import {RunConsole} from '../components/RunConsole.tsx'
import {api, errorMessage} from '../lib/api.ts'
import {buildArgv, emptyForm, type FormValues, missingRequired, shellQuote, splitArgs} from '../lib/argv.ts'
import {useRunFinished} from '../lib/runs.tsx'

const SOURCE_BADGE: Record<CommandMeta['source'], null | string> = {api: 'API', jit: 'not installed', mcp: 'MCP', plugin: null}

// Form values survive switching between commands for the session.
const formMemory = new Map<string, FormValues>()

type Props = {ctx: AppContext; selected: null | string; setSelected: (id: null | string) => void}

export function CommandsPage({ctx, selected, setSelected}: Props) {
  const [query, setQuery] = useState('')
  const [topic, setTopic] = useState('')
  const [result, setResult] = useState<null | SearchResult>(null)
  const [error, setError] = useState<null | string>(null)
  const searchRef = useRef<HTMLInputElement>(null)

  // Search runs natively in Rust on every keystroke — no debounce needed.
  useEffect(() => {
    let stale = false
    api
      .search(query, 2000)
      .then((r) => !stale && setResult(r))
      .catch((error_) => !stale && setError(errorMessage(error_)))
    return () => {
      stale = true
    }
  }, [query, ctx.catalog])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement
      if ((e.key === 'k' && (e.metaKey || e.ctrlKey)) || (e.key === '/' && !typing)) {
        e.preventDefault()
        searchRef.current?.focus()
        searchRef.current?.select()
      }
    }

    globalThis.addEventListener('keydown', onKey)
    return () => globalThis.removeEventListener('keydown', onKey)
  }, [])

  const commands = useMemo(
    () => (result?.commands ?? []).filter((c) => !topic || c.topic === topic),
    [result, topic],
  )

  return (
    <div className="split">
      <aside className="list-pane">
        <div className="search">
          <input
            autoFocus
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && commands[0]) setSelected(commands[0].id)
            }}
            placeholder="Search commands…  (⌘K)"
            ref={searchRef}
            spellCheck={false}
            value={query}
          />
          <select onChange={(e) => setTopic(e.target.value)} value={topic}>
            <option value="">All topics</option>
            {ctx.catalog?.topics.map((t) => (
              <option key={t.id} title={t.description ?? undefined} value={t.id}>
                {t.id}
              </option>
            ))}
          </select>
        </div>
        <button className={selected ? 'cmd raw-link' : 'cmd raw-link active'} onClick={() => setSelected(null)} type="button">
          <span className="cmd-id">› Type a command line…</span>
        </button>
        <div className="list-meta muted">
          {commands.length} {commands.length === 1 ? 'command' : 'commands'}
          {result && query && ` · matched in ${result.searchMicros} µs`}
          {(ctx.catalog?.blockedCount ?? 0) > 0 && ` · ${ctx.catalog?.blockedCount} hidden by permissions`}
        </div>
        {error && <div className="banner error">{error}</div>}
        <ul className="cmd-list">
          {commands.map((c) => (
            <li key={c.id}>
              <button className={selected === c.id ? 'cmd active' : 'cmd'} onClick={() => setSelected(c.id)} type="button">
                <span className="cmd-id">
                  {c.displayId}
                  {SOURCE_BADGE[c.source] && <b className={`badge src-${c.source}`}>{SOURCE_BADGE[c.source]}</b>}
                </span>
                <span className="cmd-summary">{c.summary ?? c.description?.split('\n')[0] ?? ''}</span>
              </button>
            </li>
          ))}
        </ul>
      </aside>
      <section className="detail-pane">
        {selected ? <CommandDetail ctx={ctx} id={selected} key={selected} /> : <RawRunner ctx={ctx} />}
      </section>
    </div>
  )
}

function CommandDetail({ctx, id}: {ctx: AppContext; id: string}) {
  const [command, setCommand] = useState<CommandMeta | null>(null)
  const [error, setError] = useState<null | string>(null)
  const [values, setValues] = useState<FormValues>(() => formMemory.get(id) ?? emptyForm())
  const [runId, setRunId] = useState<null | number>(null)

  useEffect(() => {
    api.command(id).then(setCommand, (error_) => setError(errorMessage(error_)))
  }, [id, ctx.catalog])

  const update = (v: FormValues) => {
    formMemory.set(id, v)
    setValues(v)
  }

  // A JIT command's first run installs its plugin; refresh so it shows as installed.
  useRunFinished(
    runId,
    useCallback(() => {
      if (command?.source === 'jit') ctx.refresh()
    }, [command, ctx]),
  )

  const run = useCallback(async () => {
    if (!command) return
    try {
      setError(null)
      setRunId(await api.runCommand(command.id, buildArgv(command, values)))
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }, [command, values])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        run()
      }
    }

    globalThis.addEventListener('keydown', onKey)
    return () => globalThis.removeEventListener('keydown', onKey)
  }, [run])

  if (error && !command) return <div className="banner error">{error}</div>
  if (!command) return <p className="muted pad">Loading…</p>

  const bin = ctx.catalog?.bin ?? 'sdkck'
  const argv = [...command.displayId.split(' '), ...buildArgv(command, values)]
  const missing = missingRequired(command, values)
  return (
    <div className="detail">
      <header className="detail-head">
        <h2>{command.displayId}</h2>
        <div className="chips">
          <span className="chip" title="Plugin">
            {command.pluginName}
          </span>
          {command.aliases.length > 0 && <span className="chip">alias: {command.aliases.map((a) => a.replaceAll(':', ' ')).join(', ')}</span>}
          {SOURCE_BADGE[command.source] && <span className={`chip src-${command.source}`}>{SOURCE_BADGE[command.source]}</span>}
          {!command.permission.allowed && <span className="chip bad">blocked by “{command.permission.pattern ?? 'no allow rule'}”</span>}
        </div>
      </header>
      {command.summary && <p className="lead">{command.summary}</p>}
      {command.description && command.description !== command.summary && <p className="description">{command.description}</p>}
      {command.source === 'jit' && (
        <div className="banner info">
          <span>
            {command.pluginName} isn’t installed yet. The first run installs it automatically (one time, can take a minute).
          </span>
          <button className="ghost small" onClick={async () => setRunId(await api.installPlugin(command.pluginName))} type="button">
            Install now
          </button>
        </div>
      )}
      {command.examples.length > 0 && (
        <details className="examples">
          <summary>Examples</summary>
          {command.examples.map((ex) => (
            <code key={ex}>{ex}</code>
          ))}
        </details>
      )}

      <CommandForm command={command} onChange={update} values={values} />

      <div className="run-bar">
        <code className="preview" title="Equivalent terminal command">
          {bin} {shellQuote(argv)}
        </code>
        <button className="ghost" onClick={() => navigator.clipboard.writeText(`${bin} ${shellQuote(argv)}`)} type="button">
          Copy
        </button>
        <button className="ghost" onClick={() => update(emptyForm())} type="button">
          Clear
        </button>
        <button
          className="primary"
          disabled={missing.length > 0}
          onClick={run}
          title={missing.length > 0 ? `Fill in: ${missing.join(', ')}` : 'Run (⌘↵)'}
          type="button"
        >
          Run
        </button>
      </div>
      {error && <div className="banner error">{error}</div>}
      <RunConsole bin={bin} runId={runId} />
    </div>
  )
}

/** Free-form runner shown when no command is selected. */
function RawRunner({ctx}: {ctx: AppContext}) {
  const [line, setLine] = useState('')
  const [runId, setRunId] = useState<null | number>(null)
  const [error, setError] = useState<null | string>(null)
  const bin = ctx.catalog?.bin ?? 'sdkck'
  const submit = async () => {
    let argv = splitArgs(line)
    if (argv[0] === bin) argv = argv.slice(1)
    if (argv.length === 0) return
    try {
      setError(null)
      setRunId(await api.runRaw(argv))
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }

  return (
    <div className="detail">
      <h2>Run anything</h2>
      <p className="muted">
        Pick a command on the left for a guided form, or type any {bin} command line here — exactly as you would in a terminal.
      </p>
      <form
        className="raw"
        onSubmit={(e) => {
          e.preventDefault()
          submit()
        }}
      >
        <span className="prompt">{bin}</span>
        <input onChange={(e) => setLine(e.target.value)} placeholder="jira issue get SS-1 --toon" spellCheck={false} value={line} />
        <button className="primary" type="submit">
          Run
        </button>
      </form>
      {error && <div className="banner error">{error}</div>}
      <RunConsole bin={bin} runId={runId} />
      {!runId && ctx.catalog && ctx.catalog.warnings.length > 0 && (
        <div className="banner warn">
          <b>Catalog notes</b>
          <ul>
            {ctx.catalog.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
