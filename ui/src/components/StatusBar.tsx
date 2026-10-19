import type {CatalogSummary, Status, WorkerState} from '../lib/types.ts'

const WORKER_LABEL: Record<WorkerState, string> = {
  booting: 'warming up',
  busy: 'busy',
  failed: 'restarting',
  off: 'off',
  ready: 'ready',
}

export function StatusBar({catalog, status}: {catalog: CatalogSummary | null; status: Status}) {
  const vault = status.interception
  const warmOn = status.settings.warmRunner && vault.state === 'off'
  return (
    <footer className="statusbar">
      <span title={status.node?.path}>Node {status.node?.version ?? '—'}</span>
      <span title={status.sdkck?.root}>
        sdkck {status.sdkck?.version ?? '—'} <em>({status.sdkck?.origin})</em>
      </span>
      {catalog && (
        <span title="Built natively from oclif manifests and stores, no Node process">
          {catalog.commandCount} commands · indexed in {(catalog.buildMicros / 1000).toFixed(1)} ms
        </span>
      )}
      <span className={`pill worker-${warmOn ? status.worker : 'off'}`} title="Pre-booted Node worker that runs commands without a cold start">
        Warm runner: {warmOn ? WORKER_LABEL[status.worker] : vault.state === 'off' ? 'disabled' : 'off (Agent Vault)'}
      </span>
      {vault.state !== 'off' && (
        <span className={`pill ${vault.state === 'misconfigured' ? 'bad' : 'good'}`}>
          Agent Vault:{' '}
          {vault.state === 'platform' ? `platform via ${vault.proxy}` : vault.state === 'broker' ? `vault ${vault.vault}` : vault.message}
        </span>
      )}
    </footer>
  )
}
