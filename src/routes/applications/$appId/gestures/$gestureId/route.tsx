import { createFileRoute, Outlet, useParams } from "@tanstack/react-router"
import { GesturePanelLayout } from "@/components/applications/app-settings-layout"

export const Route = createFileRoute(
  "/applications/$appId/gestures/$gestureId",
)({
  component: ApplicationsGestureLayout,
})

function ApplicationsGestureLayout() {
  const { appId, gestureId } = useParams({
    from: "/applications/$appId/gestures/$gestureId",
  })

  return (
    <GesturePanelLayout appId={appId} selectedGestureId={gestureId}>
      <Outlet />
    </GesturePanelLayout>
  )
}
