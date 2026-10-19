// Turning form state into the argv sdkck parses, and back to a shell line.
// Pure functions (no Tauri imports) so `npm test` can run them under Node.

import type {CommandMeta} from './types.ts'

export type FormValues = {
  args: Record<string, string>
  /** Booleans for boolean flags; raw text for option flags (one value per line when multiple). */
  flags: Record<string, boolean | string>
}

export const emptyForm = (): FormValues => ({args: {}, flags: {}})

type BooleanFlag = Pick<CommandMeta['flags'][number], 'allowNo' | 'default'>

/** A boolean flag's effective value: the user's choice, else the flag's default. */
export const booleanOn = (flag: BooleanFlag, value: boolean | string | undefined): boolean =>
  typeof value === 'boolean' ? value : flag.default === true

/** Whether the form can change a boolean flag (a default-on flag needs `--no-`). */
export const booleanEditable = (flag: BooleanFlag): boolean => flag.default !== true || flag.allowNo

/** Build the argv that follows the command id. */
export function buildArgv(command: Pick<CommandMeta, 'args' | 'flags'>, values: FormValues): string[] {
  const flagArgv: string[] = []
  for (const flag of command.flags) {
    const value = values.flags[flag.name]
    if (flag.kind === 'boolean') {
      // A default-on flag is already on; turning one off needs its --no- form.
      if (value === true && flag.default !== true) flagArgv.push(`--${flag.name}`)
      else if (value === false && flag.allowNo) flagArgv.push(`--no-${flag.name}`)
      continue
    }

    if (typeof value !== 'string' || value.trim() === '') continue
    const parts = flag.multiple
      ? value
          .split('\n')
          .map((s) => s.trim())
          .filter(Boolean)
      : [value]
    for (const part of parts) flagArgv.push(`--${flag.name}`, part)
  }

  // Positionals in declaration order; a gap before a filled one is kept as ''.
  const positional = command.args.map((arg) => values.args[arg.name] ?? '')
  while (positional.length > 0 && positional.at(-1) === '') positional.pop()

  // `--` stops flag parsing, so a positional value like "-1" is not misread.
  const needsSeparator = positional.some((v) => v.startsWith('-'))
  return [...flagArgv, ...(needsSeparator ? ['--'] : []), ...positional]
}

/** Names of required args/flags that are still empty. */
export function missingRequired(command: Pick<CommandMeta, 'args' | 'flags'>, values: FormValues): string[] {
  const missing: string[] = []
  for (const arg of command.args) {
    if (arg.required && !(values.args[arg.name] ?? '').trim()) missing.push(arg.name)
  }

  for (const flag of command.flags) {
    if (!flag.required) continue
    const value = values.flags[flag.name]
    const empty = flag.kind === 'boolean' ? !booleanOn(flag, value) : typeof value !== 'string' || !value.trim()
    if (empty) missing.push(`--${flag.name}`)
  }

  return missing
}

const SAFE = /^[\w@%+=:,./-]+$/

/** POSIX-shell quoting, for "copy as command". */
export function shellQuote(argv: string[]): string {
  return argv.map((a) => (a !== '' && SAFE.test(a) ? a : `'${a.replaceAll("'", `'\\''`)}'`)).join(' ')
}

/**
 * Split a typed command line into argv, honouring '…' and "…". A backslash
 * only escapes a quote or (outside quotes) whitespace; anywhere else it is
 * literal, so pasted Windows paths like C:\Users\me survive.
 */
export function splitArgs(line: string): string[] {
  const out: string[] = []
  let current = ''
  let quote: '"' | "'" | null = null
  let started = false
  for (let i = 0; i < line.length; i++) {
    const c = line[i]
    if (quote) {
      if (c === quote) quote = null
      else if (c === '\\' && quote === '"' && line[i + 1] === '"') current += line[++i]
      else current += c
    } else if (c === '"' || c === "'") {
      quote = c
      started = true
    } else if (c === '\\' && i + 1 < line.length && /[\s'"]/.test(line[i + 1])) {
      current += line[++i]
      started = true
    } else if (/\s/.test(c)) {
      if (started) out.push(current)
      current = ''
      started = false
    } else {
      current += c
      started = true
    }
  }

  if (started) out.push(current)
  return out
}
