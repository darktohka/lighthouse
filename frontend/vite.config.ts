import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      // The registry control plane and OCI endpoints share one origin in
      // production. In development we proxy them so the httpOnly access-token
      // cookie is set on the Vite origin and sent on every /api request.
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/v2': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
})
