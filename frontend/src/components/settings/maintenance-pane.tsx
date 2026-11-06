import { ClearDone } from "@/components/cards/clear-done"

// MaintenancePane is Settings > Maintenance (card #138): clear done cards
// by age or all of them. One by one, and deleting for good, are on the
// Closed view's Select.
export function MaintenancePane() {
  return (
    <div className="grid gap-4">
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">Clear done cards</h3>
        <p className="text-[12.5px] text-muted-foreground">
          Cleared cards leave the app's lists, here and in your spaces. They
          stay in Irori's log, so agents still find them with search and show.
          To clear or delete single cards, use Select on Closed.
        </p>
        <ClearDone />
      </section>
    </div>
  )
}
