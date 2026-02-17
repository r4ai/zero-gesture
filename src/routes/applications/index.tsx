import { createFileRoute, redirect } from "@tanstack/react-router"
import { getGestureId } from "@/components/applications/app-settings-layout"
import { DEFAULT_BINDINGS } from "@/types/config"

export const Route = createFileRoute("/applications/")({
  beforeLoad: () => {
    throw redirect({
      to: "/applications/$appId/gestures/$gestureId",
      params: {
        appId: "default",
        gestureId: getGestureId(DEFAULT_BINDINGS[0]),
      },
    })
  },
})
