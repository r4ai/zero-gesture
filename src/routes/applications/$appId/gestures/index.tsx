import { createFileRoute, Navigate, useParams } from "@tanstack/react-router"
import {
  AppSettingsLayout,
  getGestureId,
} from "@/components/applications/app-settings-layout"
import { useConfigDraft } from "@/contexts/config-draft"

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
        params={{ appId, gestureId: getGestureId(firstGesture) }}
        search={{ tab: "gesture" }}
        replace
      />
    )
  }

  return (
    <AppSettingsLayout appId={appId} selectedGestureId="">
      <div className="flex h-full items-center justify-center">
        <p className="text-[14px] text-foreground-subtle">Gesture not found</p>
      </div>
    </AppSettingsLayout>
  )
}
