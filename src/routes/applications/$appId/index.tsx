import { createFileRoute, redirect } from "@tanstack/react-router"
import {
  GESTURES,
  getGestureId,
} from "@/components/applications/app-settings-layout"

export const Route = createFileRoute("/applications/$appId/")({
  beforeLoad: ({ params }) => {
    throw redirect({
      to: "/applications/$appId/gestures/$gestureId",
      params: {
        appId: params.appId,
        gestureId: getGestureId(GESTURES[0]),
      },
    })
  },
})
