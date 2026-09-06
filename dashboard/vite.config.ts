import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'path'

const backendTarget = process.env.MODELPORT_VITE_PROXY_TARGET || 'http://127.0.0.1:38082'

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
      // Mock modules only export fixtures. Their generated sample data need
      // not execute when production has removed all mock-mode branches.
      treeshake: { moduleSideEffects: (id) => !id.includes('/src/mock/') },
      output: {
        // Keep shared React code stable; let route imports own their optional
        // dependencies so the login page does not preload the chart library.
        manualChunks(id) {
          if (/\/node_modules\/(react|react-dom|react-router|react-router-dom)\//.test(id)) return 'react-vendor'
        },
      },
    },
  },
  server: {
    port: 33002,
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
