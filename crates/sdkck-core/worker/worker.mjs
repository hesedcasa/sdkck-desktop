// sdkck Desktop warm runner.
//
// One long-lived Node process that keeps sdkck's @oclif/core, plugins and their
// module graphs loaded, then executes each requested invocation in-process
// through oclif's own `run()` — the same entry point `bin/run.js` uses — so
// init/prerun hooks (permission enforcement, telemetry, file logging) run for
// every command exactly as they do from a terminal. What it saves is the Node
// boot and module loading every `sdkck …` invocation pays.
//
// Protocol: requests are JSON lines on stdin ({id, argv}). Replies are JSON
// lines on stdout prefixed with PREFIX; anything else that reaches the real
// stdout (a subprocess inheriting it) belongs to the running command.
//
// Usage: node worker.mjs <sdkck package root>

import {createRequire} from 'node:module'
import {join} from 'node:path'
import {createInterface} from 'node:readline'
import {Readable} from 'node:stream'
import {pathToFileURL} from 'node:url'

const PREFIX = '\u001ESDKCKD '
const root = process.argv[2]
if (!root) {
  process.stderr.write('usage: worker.mjs <sdkck root>\n')
  process.exit(64)
}

const realStdoutWrite = process.stdout.write.bind(process.stdout)
const realExit = process.exit.bind(process)
const send = (msg) => realStdoutWrite(PREFIX + JSON.stringify(msg) + '\n')

// Resolve sdkck's own copies, exactly as bin/run.js would see them.
const requireFromRoot = createRequire(join(root, 'package.json'))
const importFromRoot = (spec) => import(pathToFileURL(requireFromRoot.resolve(spec)).href)
const {Command, Config, run} = await importFromRoot('@oclif/core')

// ── Mirror of bin/run.js's Config.load patch (keep in sync) ───────────────────
let registerApiCommands = async () => {}
try {
  ;({registerApiCommands} = await importFromRoot('@hesed/api2cli'))
} catch {
  // older sdkck without api2cli
}

async function registerMcpClientCommands(config) {
  try {
    // pathToFileURL rather than bin/run.js's `file://${dataDir}/` template, so
    // a data dir containing `#` or `%` still resolves.
    const pluginPath = pathToFileURL(join(config.dataDir, 'node_modules', '@hesed', 'mcp-client', 'dist', 'index.js')).href
    const {registerMcpClientCommands: register} = await import(pluginPath)
    await register(config)
  } catch {
    // plugin not installed yet
  }
}

const PERSISTED = Symbol('sdkck.persistedDynamicCommands')
function persistDynamicCommands(config) {
  const rootPlugin = config.rootPlugin
  const owned = new Set(config.getPluginsList().flatMap((p) => p.commands.flatMap((c) => [c.id, ...(c.aliases ?? [])])))
  const dynamic = config.commands.filter((c) => !owned.has(c.id))
  rootPlugin.commands.push(...dynamic)
  rootPlugin[PERSISTED] = [...(rootPlugin[PERSISTED] ?? []), ...dynamic]
}

function unpersistDynamicCommands(plugin) {
  const persisted = new Set(plugin?.[PERSISTED])
  if (persisted.size === 0) return
  plugin.commands = plugin.commands.filter((c) => !persisted.has(c))
  plugin[PERSISTED] = []
}

const originalLoad = Config.load.bind(Config)
Config.load = async (...args) => {
  unpersistDynamicCommands(args[0]?.rootPlugin)
  const config = await originalLoad(...args)
  await registerApiCommands(config).catch(() => {})
  await registerMcpClientCommands(config)
  persistDynamicCommands(config)
  return config
}
// ──────────────────────────────────────────────────────────────────────────────

class WorkerExit extends Error {
  constructor(code) {
    super(`process.exit(${code})`)
    this.code = code
  }
}

const runFile = pathToFileURL(join(root, 'bin', 'run.js')).href
let current

// Init hooks that patch globals per invocation (setup-file-logging wraps
// Command.prototype.warn/error and console.warn/error) would stack one wrapper
// per run in a long-lived process. Restore the originals after every run.
const snapshot = () => ({
  cmdError: Command.prototype.error,
  cmdWarn: Command.prototype.warn,
  conError: console.error,
  conWarn: console.warn,
  conLog: console.log,
})
const restore = (s) => {
  Command.prototype.error = s.cmdError
  Command.prototype.warn = s.cmdWarn
  console.error = s.conError
  console.warn = s.conWarn
  console.log = s.conLog
}

function capture(id, stream, silent) {
  return (chunk, encoding, cb) => {
    const data = typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString('utf8')
    if (data && !silent) send({d: data, id, s: stream, t: 'out'})
    const callback = typeof encoding === 'function' ? encoding : cb
    if (typeof callback === 'function') callback()
    return true
  }
}

function exitCodeOf(error) {
  if (error instanceof WorkerExit) return error.code
  const code = error?.oclif?.exit
  return typeof code === 'number' ? code : 1
}

async function execute({argv, id}, silent = false) {
  if (!silent) current = id
  const saved = snapshot()
  const stdoutWrite = process.stdout.write
  const stderrWrite = process.stderr.write
  const stdinDescriptor = Object.getOwnPropertyDescriptor(process, 'stdin')
  process.stdout.write = capture(id, 'stdout', silent)
  process.stderr.write = capture(id, 'stderr', silent)
  // Commands must never read the protocol channel; give them an empty stdin,
  // as the cold path does.
  const emptyStdin = Readable.from([])
  emptyStdin.isTTY = false
  Object.defineProperty(process, 'stdin', {configurable: true, get: () => emptyStdin})
  process.exit = (code) => {
    throw new WorkerExit(code ?? process.exitCode ?? 0)
  }

  process.exitCode = undefined
  let code = 0
  try {
    await run(argv, runFile)
    code = typeof process.exitCode === 'number' ? process.exitCode : 0
  } catch (error) {
    code = exitCodeOf(error)
    // Mirror oclif's handle(): an ExitError is silent, anything else prints.
    const silent = error instanceof WorkerExit || error?.code === 'EEXIT'
    if (!silent && code !== 0) {
      const message = error?.oclif?.exit === undefined && error?.stack ? error.stack : `Error: ${error?.message ?? error}`
      process.stderr.write(message.endsWith('\n') ? message : message + '\n')
    }
  } finally {
    process.stdout.write = stdoutWrite
    process.stderr.write = stderrWrite
    process.exit = realExit
    Object.defineProperty(process, 'stdin', stdinDescriptor)
    process.exitCode = undefined
    restore(saved)
    current = undefined
  }

  if (!silent) send({code, id, t: 'exit'})
}

// A stray async failure leaves the process in an unknown state: finish the
// current run as failed and exit, so the app starts a fresh worker.
const fatal = (error) => {
  if (current !== undefined) {
    send({d: `${error?.stack ?? error}\n`, id: current, s: 'stderr', t: 'out'})
    send({code: 1, id: current, t: 'exit'})
  }

  send({message: String(error?.message ?? error), t: 'fatal'})
  realExit(70)
}

process.on('uncaughtException', fatal)
process.on('unhandledRejection', fatal)

// Warm the module graph so the first real run is fast: a bare `--version`
// loads the config and runs every init hook (their module imports are most of
// a first run's cost) but short-circuits before runCommand, so telemetry does
// not count it as a command.
await execute({argv: ['--version'], id: 0}, true).catch(() => {})

let queue = Promise.resolve()
const lines = createInterface({input: process.stdin})
lines.on('line', (line) => {
  if (!line.trim()) return
  let request
  try {
    request = JSON.parse(line)
  } catch {
    return
  }

  queue = queue.then(() => execute(request))
})
lines.on('close', () => queue.then(() => realExit(0)))

send({pid: process.pid, t: 'ready'})
