import { createFileRoute } from "@tanstack/react-router"

function GeneralPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-bold text-2xl tracking-tight">General</h2>
        <p className="text-muted-foreground">
          Configure general settings for Zero Gesture.
        </p>
      </div>
      <div className="rounded-lg border p-8 text-center">
        <p className="text-muted-foreground">
          General settings will be implemented here.
        </p>
      </div>
    </div>
  )
}

export const Route = createFileRoute("/general/")({
  component: GeneralPage,
})
