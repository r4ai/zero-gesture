import { createFileRoute, redirect } from "@tanstack/react-router"

export const Route = createFileRoute("/applications/")({
  beforeLoad: () => {
    throw redirect({
      to: "/applications/$appId/edit",
      params: {
        appId: "default",
      },
    })
  },
})
