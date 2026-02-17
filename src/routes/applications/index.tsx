import { createFileRoute, redirect } from "@tanstack/react-router"
import {
  GESTURES,
  getGestureId,
} from "@/components/applications/app-settings-layout"

export const Route = createFileRoute("/applications/")({
  beforeLoad: () => {
    throw redirect({
      to: "/applications/$appId/gestures/$gestureId",
      params: { appId: "default", gestureId: getGestureId(GESTURES[0]) },
    })
  },
})
