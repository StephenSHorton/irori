import { resolve } from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, type Plugin } from "vite"

// The dev server says which folder it serves at /__preview-root, so
// preview:look and the shot scripts can tell their own preview from another
// worktree's on the same port (frontend/PREVIEW.md, card #610). Dev only.
const previewRoot: Plugin = {
  name: "preview-root",
  configureServer(server) {
    server.middlewares.use("/__preview-root", (_req, res) => {
      res.setHeader("Content-Type", "text/plain")
      res.end(server.config.root)
    })
  },
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss(), previewRoot],
  resolve: {
    alias: [{ find: "@", replacement: resolve(import.meta.dirname, "./src") }],
  },
  server: {
    host: "127.0.0.1",
    port: Number(process.env.WAILS_VITE_PORT) || 9245,
    strictPort: true,
  },
})
