import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Served from https://stephenshorton.github.io/irori/, so assets live under /irori/.
export default defineConfig({
  base: '/irori/',
  plugins: [react(), tailwindcss()],
  // The 3D hero (three, r3f, drei, postprocessing) is one lazy chunk loaded after first paint.
  build: { chunkSizeWarningLimit: 1400 },
  optimizeDeps: {
    include: ['stats.js', 'three', '@react-three/fiber', '@react-three/drei', '@react-three/postprocessing'],
  },
  server: { host: '127.0.0.1', port: 43190, strictPort: true },
  preview: { host: '127.0.0.1', port: 43190, strictPort: true },
})
