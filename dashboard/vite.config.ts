import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'path'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/v1': 'http://127.0.0.1:17878',
      '/health': 'http://127.0.0.1:17878',
      '/metrics': 'http://127.0.0.1:17878',
      '/admin': 'http://127.0.0.1:17878',
    },
  },
})
