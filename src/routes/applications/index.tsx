import { createFileRoute, Navigate } from "@tanstack/react-router"
import { getGestureId } from "@/components/applications/app-settings-layout"
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
        params={{ appId: "default", gestureId: getGestureId(firstGesture) }}
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
