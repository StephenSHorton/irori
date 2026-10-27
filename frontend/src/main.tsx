import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import App from "./App.tsx"
import { ThemeProvider } from "@/components/theme-provider.tsx"
import { TooltipProvider } from "@/components/ui/tooltip"

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {/* Light only: Irori has no dark palette yet (its .dark tokens copy the
        light ones), and under a dark OS theme mokei's dark: styles would
        grey a light page. The fresh storage key drops any stored "dark". */}
    <ThemeProvider defaultTheme="light" storageKey="irori-theme">
      <TooltipProvider delay={300}>
        <App />
      </TooltipProvider>
    </ThemeProvider>
  </StrictMode>
)
