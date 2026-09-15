import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

const tauriDevHost = process.env.TAURI_DEV_HOST

export default defineConfig({
  clearScreen: false,
  plugins: [vue()],
  server: {
    host: tauriDevHost || '127.0.0.1',
    port: 1420,
    strictPort: true,
    hmr: tauriDevHost
      ? {
          protocol: 'ws',
          host: tauriDevHost,
          port: 1421,
          overlay: false
        }
      : { overlay: false },
    watch: {
      ignored: ['**/src-tauri/**']
    }
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    target: 'es2020',
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG)
  }
})
