import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import App from "./App.tsx"
import { TooltipProvider } from "@/components/ui/tooltip"

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {/* Dark only, like Toru: index.html carries class="dark", so there is
        no theme provider and no light/dark toggle on this branch. */}
    <TooltipProvider delay={300}>
      <App />
    </TooltipProvider>
  </StrictMode>
)
