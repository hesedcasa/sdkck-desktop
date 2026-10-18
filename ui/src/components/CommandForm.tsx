import type {FormValues} from '../lib/argv.ts'
import type {CommandMeta} from '../lib/types.ts'

type Props = {
  command: CommandMeta
  onChange: (values: FormValues) => void
  values: FormValues
}

function placeholderOf(value: unknown): string | undefined {
  if (value === null || value === undefined || value === '') return undefined
  return `default: ${typeof value === 'string' ? value : JSON.stringify(value)}`
}

/** Form controls for a command's positional args and flags. */
export function CommandForm({command, onChange, values}: Props) {
  const setArg = (name: string, v: string) => onChange({...values, args: {...values.args, [name]: v}})
  const setFlag = (name: string, v: boolean | string) => onChange({...values, flags: {...values.flags, [name]: v}})

  const required = command.flags.filter((f) => f.required)
  const optional = command.flags.filter((f) => !f.required)

  return (
    <div className="form">
      {command.args.length > 0 && <h4>Arguments</h4>}
      {command.args.map((arg) => (
        <label className="field" key={`arg-${arg.name}`}>
          <span className="field-name">
            {arg.name}
            {arg.required && <b className="req">required</b>}
          </span>
          {arg.options ? (
            <select onChange={(e) => setArg(arg.name, e.target.value)} value={values.args[arg.name] ?? ''}>
              <option value="">—</option>
              {arg.options.map((o) => (
                <option key={o}>{o}</option>
              ))}
            </select>
          ) : (
            <input
              onChange={(e) => setArg(arg.name, e.target.value)}
              placeholder={placeholderOf(arg.default)}
              spellCheck={false}
              value={values.args[arg.name] ?? ''}
            />
          )}
          {arg.description && arg.description !== arg.name && <small>{arg.description}</small>}
        </label>
      ))}

      {[...required, ...optional].length > 0 && <h4>Flags</h4>}
      {[...required, ...optional].map((flag) => {
        const label = (
          <span className="field-name">
            --{flag.name}
            {flag.char && <em> -{flag.char}</em>}
            {flag.required && <b className="req">required</b>}
            {flag.multiple && <b className="tag">repeatable</b>}
          </span>
        )
        if (flag.kind === 'boolean') {
          return (
            <label className="field check" key={`flag-${flag.name}`}>
              <input checked={values.flags[flag.name] === true} onChange={(e) => setFlag(flag.name, e.target.checked)} type="checkbox" />
              <div>
                {label}
                {flag.description && <small>{flag.description}</small>}
              </div>
            </label>
          )
        }

        const value = typeof values.flags[flag.name] === 'string' ? (values.flags[flag.name] as string) : ''
        return (
          <label className="field" key={`flag-${flag.name}`}>
            {label}
            {flag.options && !flag.multiple ? (
              <select onChange={(e) => setFlag(flag.name, e.target.value)} value={value}>
                <option value="">{placeholderOf(flag.default) ?? '—'}</option>
                {flag.options.map((o) => (
                  <option key={o}>{o}</option>
                ))}
              </select>
            ) : flag.multiple ? (
              <textarea
                onChange={(e) => setFlag(flag.name, e.target.value)}
                placeholder="One value per line"
                rows={2}
                spellCheck={false}
                value={value}
              />
            ) : (
              <input onChange={(e) => setFlag(flag.name, e.target.value)} placeholder={placeholderOf(flag.default)} spellCheck={false} value={value} />
            )}
            {flag.description && <small>{flag.description}</small>}
          </label>
        )
      })}
      {command.args.length === 0 && command.flags.length === 0 && <p className="muted">This command takes no input.</p>}
    </div>
  )
}
