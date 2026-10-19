import {useEffect, useState} from 'react'

import type {AppContext} from '../App.tsx'
import type {CommandMeta, Decision, PermissionView} from '../lib/types.ts'

import {api, errorMessage} from '../lib/api.ts'

const REASON: Record<Decision['reason'], string> = {
  'allow-rule': 'allowed by',
  'disallow-rule': 'blocked by',
  'no-allow-rule': 'blocked: no allow rule matches',
  'permission-command': 'always allowed (permission commands are never gated)',
}

export function PermissionsPage({ctx}: {ctx: AppContext}) {
  const [view, setView] = useState<null | PermissionView>(null)
  const [blocked, setBlocked] = useState<CommandMeta[]>([])
  const [pattern, setPattern] = useState('')
  const [kind, setKind] = useState<'allow' | 'deny'>('deny')
  const [probe, setProbe] = useState('')
  const [decision, setDecision] = useState<Decision | null>(null)
  const [error, setError] = useState<null | string>(null)

  const load = async (next?: PermissionView) => {
    try {
      setView(next ?? (await api.permissions()))
      await ctx.refresh()
      setBlocked(await api.blockedCommands())
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }

  useEffect(() => {
    load()
  }, []) // once, on open

  useEffect(() => {
    if (!probe.trim()) {
      setDecision(null)
      return
    }

    api.checkPermission(probe.trim()).then(setDecision, () => setDecision(null))
  }, [probe, view])

  const act = async (f: () => Promise<PermissionView>) => {
    try {
      setError(null)
      await load(await f())
    } catch (error_) {
      setError(errorMessage(error_))
    }
  }

  const rules = (k: 'allow' | 'deny') => (k === 'allow' ? view?.config.allowRules : view?.config.denyRules) ?? []

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>Permissions</h1>
          <p className="muted">
            Control which commands people and agents can run. Rules are shared with the CLI (<code>{view?.path}</code>): a deny rule always wins,
            and a command must match an allow rule. Patterns use spaces — <code>jira</code> or <code>jira *</code> covers the whole topic,{' '}
            <code>jira issue delete</code> one command.
          </p>
        </div>
        <button className="ghost" onClick={() => act(api.resetPermissions)} type="button">
          Reset to allow all
        </button>
      </header>
      {error && <div className="banner error">{error}</div>}
      {view?.error && <div className="banner error">{view.error} — every command is blocked until this is fixed.</div>}
      {view && !view.exists && <div className="banner info">No rules yet — everything is allowed.</div>}

      <form
        className="inline"
        onSubmit={(e) => {
          e.preventDefault()
          if (pattern.trim()) act(() => api.addPermissionRule(kind, pattern)).then(() => setPattern(''))
        }}
      >
        <select onChange={(e) => setKind(e.target.value as 'allow' | 'deny')} value={kind}>
          <option value="deny">Block</option>
          <option value="allow">Allow</option>
        </select>
        <input
          onChange={(e) => setPattern(e.target.value)}
          placeholder="e.g. jira issue delete, mysql, *"
          spellCheck={false}
          value={pattern}
        />
        <button className="primary" disabled={!pattern.trim()} type="submit">
          Add rule
        </button>
      </form>

      <div className="columns">
        {(['deny', 'allow'] as const).map((k) => (
          <section className="rule-list" key={k}>
            <h3>{k === 'deny' ? 'Blocked' : 'Allowed'}</h3>
            {rules(k).length === 0 && <p className="muted">None.</p>}
            <ul>
              {rules(k).map((r) => (
                <li key={r.pattern}>
                  <code>{r.pattern}</code>
                  <button className="ghost small" onClick={() => act(() => api.removePermissionRule(k, r.pattern))} type="button">
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>

      <h3>Test a command</h3>
      <input className="wide" onChange={(e) => setProbe(e.target.value)} placeholder="jira issue delete" spellCheck={false} value={probe} />
      {decision && (
        <p className={decision.allowed ? 'good-text' : 'bad-text'}>
          {decision.allowed ? '✓' : '✕'} {REASON[decision.reason]} {decision.pattern && <code>{decision.pattern}</code>}
        </p>
      )}

      {blocked.length > 0 && (
        <>
          <h3>Currently hidden ({blocked.length})</h3>
          <div className="tags">
            {blocked.map((c) => (
              <code key={c.id} title={c.permission.pattern ? `by ${c.permission.pattern}` : 'no allow rule'}>
                {c.displayId}
              </code>
            ))}
          </div>
        </>
      )}
    </div>
  )
}
