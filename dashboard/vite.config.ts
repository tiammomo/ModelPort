import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'path'

const backendTarget = process.env.MODELPORT_VITE_PROXY_TARGET || 'http://127.0.0.1:17878'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  build: {
    chunkSizeWarningLimit: 450,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/recharts/') || id.includes('/node_modules/d3-')) {
            return 'charts-vendor'
          }
          if (id.includes('/node_modules/react/')
            || id.includes('/node_modules/react-dom/')
            || id.includes('/node_modules/react-router')) {
            return 'react-vendor'
          }
          if (id.includes('/node_modules/@tanstack/')) {
            return 'query-vendor'
          }
          if (id.includes('/node_modules/framer-motion/')
            || id.includes('/node_modules/lucide-react/')
            || id.includes('/node_modules/sonner/')
            || id.includes('/node_modules/cmdk/')) {
            return 'ui-vendor'
          }
        },
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/v1': backendTarget,
      '/livez': backendTarget,
      '/readyz': backendTarget,
      '/health': backendTarget,
      '/metrics': backendTarget,
      '/admin': backendTarget,
    },
  },
  test: {
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
})
