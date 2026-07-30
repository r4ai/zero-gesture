import { createFileRoute, Navigate } from "@tanstack/react-router"
import { useConfigDraft } from "@/contexts/config-draft"
import { getWindowsBindings } from "@/types/config"

export const Route = createFileRoute("/applications/")({
  component: ApplicationsIndexPage,
})

function ApplicationsIndexPage() {
  const { draft } = useConfigDraft()
  const defaultBindings = getWindowsBindings(draft, "default")
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
