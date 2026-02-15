import { createFileRoute } from "@tanstack/react-router"

function ApplicationsPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-bold text-2xl tracking-tight">Applications</h2>
        <p className="text-muted-foreground">
          Manage application-specific gesture settings.
        </p>
      </div>
      <div className="rounded-lg border p-8 text-center">
        <p className="text-muted-foreground">
          Applications settings will be implemented here.
        </p>
      </div>
    </div>
  )
}

export const Route = createFileRoute("/applications/")({
  component: ApplicationsPage,
})
