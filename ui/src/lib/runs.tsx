import {listen} from '@tauri-apps/api/event'
import {createContext, type ReactNode, useContext, useEffect, useMemo, useState, useSyncExternalStore} from 'react'

import type {RunEvent, RunMode} from './types.ts'

export type RunState = {
  argv: string[]
  cancelled: boolean
  code?: number
  durationMs?: number
  mode?: RunMode
  output: string
  runId: number
  running: boolean
}

type Listener = () => void

/**
 * All run output streamed from the backend, keyed by run id. Events can land
 * before the `invoke` that started the run resolves with its id, so the store
 * keeps every run it hears about rather than only ones a page asked for.
 */
class RunStore {
  private listeners = new Set<Listener>()
  private runs = new Map<number, RunState>()

  apply(event: RunEvent) {
    const prev = this.runs.get(event.runId) ?? {argv: [], cancelled: false, output: '', runId: event.runId, running: true}
    let next: RunState
    switch (event.type) {
      case 'finished': {
        next = {...prev, cancelled: event.cancelled, code: event.code, durationMs: event.durationMs, mode: event.mode, running: false}
        break
      }

      case 'output': {
        next = {...prev, output: prev.output + event.data}
        break
      }

      case 'started': {
        next = {...prev, argv: event.argv, mode: event.mode}
        break
      }
    }

    this.runs.set(event.runId, next)
    for (const l of this.listeners) l()
  }

  get(runId: null | number | undefined): RunState | undefined {
    return runId === null || runId === undefined ? undefined : this.runs.get(runId)
  }

  subscribe = (l: Listener) => {
    this.listeners.add(l)
    return () => this.listeners.delete(l)
  }
}

const RunsContext = createContext<RunStore | null>(null)

export function RunsProvider({children}: {children: ReactNode}) {
  const store = useMemo(() => new RunStore(), [])
  useEffect(() => {
    const unlisten = listen<RunEvent>('run-event', (e) => store.apply(e.payload))
    return () => {
      unlisten.then((f) => f())
    }
  }, [store])
  return <RunsContext.Provider value={store}>{children}</RunsContext.Provider>
}

/** Live state of one run; re-renders as its output streams in. */
export function useRun(runId: null | number | undefined): RunState | undefined {
  const store = useContext(RunsContext)
  if (!store) throw new Error('useRun outside RunsProvider')
  return useSyncExternalStore(store.subscribe, () => store.get(runId))
}

/** Calls `onDone` once when the given run finishes. */
export function useRunFinished(runId: null | number | undefined, onDone: (run: RunState) => void) {
  const run = useRun(runId)
  const [handled, setHandled] = useState<null | number>(null)
  useEffect(() => {
    if (run && !run.running && handled !== run.runId) {
      setHandled(run.runId)
      onDone(run)
    }
  }, [run, handled, onDone])
}
