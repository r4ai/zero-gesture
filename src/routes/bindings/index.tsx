import { createFileRoute } from "@tanstack/react-router"

function BindingsPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-bold text-2xl tracking-tight">Bindings</h2>
        <p className="text-muted-foreground">
          Configure gesture bindings and actions.
        </p>
      </div>
      <div className="rounded-lg border p-8 text-center">
        <p className="text-muted-foreground">
          Bindings settings will be implemented here.
        </p>
      </div>
    </div>
  )
}

export const Route = createFileRoute("/bindings/")({
  component: BindingsPage,
})
