// The in-app updater UI. The backend is internal/update (UpdateService).
export { UpdateBanner } from "./UpdateBanner"
export { UpdateStatusLine } from "./UpdateStatusLine"
export { useUpdater, UPDATE_STATUS_EVENT } from "./useUpdater"
export type { UpdatePhase, UpdaterState } from "./useUpdater"
