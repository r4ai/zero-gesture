import { createFileRoute, Navigate, useParams } from "@tanstack/react-router"
import { GesturePanelLayout } from "@/components/applications/app-settings-layout"
import { useConfigDraft } from "@/contexts/config-draft"
import { GestureNotFound } from "./-components/gesture-not-found"

export const Route = createFileRoute("/applications/$appId/gestures/")({
  component: GesturesEmptyPage,
})

function GesturesEmptyPage() {
  const { appId } = useParams({ from: "/applications/$appId/gestures/" })
  const { draft } = useConfigDraft()
  const firstGesture = draft.bindings[appId]?.[0]

  if (firstGesture) {
    return (
      <Navigate
        to="/applications/$appId/gestures/$gestureId"
        params={{ appId, gestureId: firstGesture.id }}
        search={{ tab: "gesture" }}
        replace
      />
    )
  }

  return (
    <GesturePanelLayout appId={appId} selectedGestureId={undefined}>
      <GestureNotFound />
    </GesturePanelLayout>
  )
}
