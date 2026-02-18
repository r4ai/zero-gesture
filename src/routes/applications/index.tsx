import { createFileRoute, Navigate } from "@tanstack/react-router"
import { useConfigDraft } from "@/contexts/config-draft"

export const Route = createFileRoute("/applications/")({
  component: ApplicationsIndexPage,
})

function ApplicationsIndexPage() {
  const { draft } = useConfigDraft()
  const defaultBindings = draft.bindings.default ?? []
  const firstGesture = defaultBindings[0]

  if (firstGesture) {
    return (
      <Navigate
        to="/applications/$appId/gestures/$gestureId"
        params={{ appId: "default", gestureId: firstGesture.id }}
        search={{ tab: "gesture" }}
        replace
      />
    )
  }

  return (
    <Navigate
      to="/applications/$appId/gestures"
      params={{ appId: "default" }}
      replace
    />
  )
}
