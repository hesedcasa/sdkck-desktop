import {useEffect, useRef} from 'react'

import {api} from '../lib/api.ts'
import {shellQuote} from '../lib/argv.ts'
import {useRun} from '../lib/runs.tsx'

/** Streaming output of one run, with its status line and actions. */
export function RunConsole({bin = 'sdkck', compact = false, runId}: {bin?: string; compact?: boolean; runId: null | number}) {
  const run = useRun(runId)
  const ref = useRef<HTMLPreElement>(null)
  useEffect(() => {
    const el = ref.current
    if (el) el.scrollTop = el.scrollHeight
  }, [run?.output])

  if (runId === null) return null
  const state = !run || run.running ? 'running' : run.cancelled ? 'cancelled' : run.code === 0 ? 'ok' : 'failed'
  return (
    <section className={`console ${compact ? 'compact' : ''}`}>
      <header>
        <span className={`dot ${state}`} />
        <code className="console-cmd">{run ? `${bin} ${shellQuote(run.argv)}` : '…'}</code>
        <span className="spacer" />
        {run?.mode && (
          <span className={`pill mode-${run.mode}`} title={run.mode === 'warm' ? 'Ran in the pre-booted worker' : 'Ran in a fresh Node process'}>
            {run.mode}
          </span>
        )}
        {run && !run.running && (
          <span className="muted">
            {run.cancelled ? 'cancelled' : `exit ${run.code}`} · {run.durationMs} ms
          </span>
        )}
        {run?.running && (
          <button className="ghost small" onClick={() => api.cancelRun(run.runId)} type="button">
            Stop
          </button>
        )}
        {run && !run.running && run.output && (
          <button className="ghost small" onClick={() => navigator.clipboard.writeText(run.output)} type="button">
            Copy output
          </button>
        )}
      </header>
      <pre ref={ref}>{run?.output || (run?.running ? 'Running…' : '(no output)')}</pre>
    </section>
  )
}
