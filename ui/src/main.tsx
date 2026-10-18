import {StrictMode} from 'react'
import {createRoot} from 'react-dom/client'

import {App} from './App.tsx'
import {RunsProvider} from './lib/runs.tsx'
import './styles.css'

createRoot(document.querySelector('#root')!).render(
  <StrictMode>
    <RunsProvider>
      <App />
    </RunsProvider>
  </StrictMode>,
)
