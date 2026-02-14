import { createFileRoute } from "@tanstack/react-router"

function StylePage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-bold text-2xl tracking-tight">Style</h2>
        <p className="text-muted-foreground">
          Customize the appearance of gesture trails and UI.
        </p>
      </div>
      <div className="rounded-lg border p-8 text-center">
        <p className="text-muted-foreground">
          Style settings will be implemented here.
        </p>
      </div>
    </div>
  )
}

export const Route = createFileRoute("/style/")({
  component: StylePage,
})
