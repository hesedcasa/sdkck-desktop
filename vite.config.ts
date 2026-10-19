import react from '@vitejs/plugin-react'
import {defineConfig} from 'vite'

// Tauri expects a fixed dev port and serves the built files from ../dist.
export default defineConfig({
  build: {emptyOutDir: true, outDir: '../dist', target: 'es2022'},
  clearScreen: false,
  plugins: [react()],
  root: 'ui',
  server: {port: 1420, strictPort: true, watch: {ignored: ['**/src-tauri/**', '**/target/**']}},
})
