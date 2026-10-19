import {Fragment, useEffect, useState} from 'react'

import type {AppContext} from '../App.tsx'
import type {HistoryEntry} from '../lib/types.ts'

import {RunConsole} from '../components/RunConsole.tsx'
import {api, errorMessage} from '../lib/api.ts'
import {shellQuote} from '../lib/argv.ts'
import {useRunStore} from '../lib/runs.tsx'

export function HistoryPage({ctx}: {ctx: AppContext}) {
  const [entries, setEntries] = useState<HistoryEntry[]>([])
  const [open, setOpen] = useState<null | number>(null)
  const [rerun, setRerun] = useState<null | number>(null)
  const [error, setError] = useState<null | string>(null)
  const bin = ctx.catalog?.bin ?? 'sdkck'
  const runs = useRunStore()

  const load = () => api.history().then(setEntries, (e) => setError(errorMessage(e)))
  useEffect(() => {
    load()
  }, [rerun])

  const warm = entries.filter((e) => e.mode === 'warm' && e.durationMs !== null)
  const cold = entries.filter((e) => e.mode === 'cold' && e.durationMs !== null)
  const avg = (xs: HistoryEntry[]) => Math.round(xs.reduce((n, e) => n + (e.durationMs ?? 0), 0) / Math.max(xs.length, 1))

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>History</h1>
          <p className="muted">
            {entries.length} runs
            {warm.length > 0 && ` · warm avg ${avg(warm)} ms`}
            {cold.length > 0 && ` · cold avg ${avg(cold)} ms`}
          </p>
        </div>
        <button className="ghost" onClick={() =>
            api.clearHistory().then(() => {
              runs.clearFinished()
              setRerun(null)
              load()
            })
          } type="button">
          Clear
        </button>
      </header>
      {error && <div className="banner error">{error}</div>}
      <RunConsole bin={bin} runId={rerun} />
      <table className="table history">
        <thead>
          <tr>
            <th>When</th>
            <th>Command</th>
            <th>Result</th>
            <th>Time</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {entries.map((e) => (
            <Fragment key={e.runId}>
              <tr className={open === e.runId ? 'open' : ''} onClick={() => setOpen(open === e.runId ? null : e.runId)}>
                <td className="muted">{new Date(e.startedAt).toLocaleString()}</td>
                <td>
                  <code>
                    {bin} {shellQuote(e.argv)}
                  </code>
                </td>
                <td>
                  {e.code === null ? (
                    <span className="muted">running</span>
                  ) : e.cancelled ? (
                    <span className="muted">cancelled</span>
                  ) : (
                    <span className={e.code === 0 ? 'good-text' : 'bad-text'}>exit {e.code}</span>
                  )}
                </td>
                <td className="muted">
                  {e.durationMs ?? '–'} ms {e.mode && <span className={`pill mode-${e.mode}`}>{e.mode}</span>}
                </td>
                <td className="right">
                  <button
                    className="ghost small"
                    onClick={async (ev) => {
                      ev.stopPropagation()
                      setRerun(await api.runRaw(e.argv))
                    }}
                    type="button"
                  >
                    Run again
                  </button>
                </td>
              </tr>
              {open === e.runId && (
                <tr className="output-row">
                  <td colSpan={5}>
                    <pre>{e.output || '(no output)'}</pre>
                    {e.truncated && <p className="muted">Output truncated in history.</p>}
                  </td>
                </tr>
              )}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  )
}
