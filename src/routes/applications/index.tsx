import { createFileRoute, redirect } from "@tanstack/react-router"

export const Route = createFileRoute("/applications/")({
  beforeLoad: () => {
    // Redirect to the first app by default
    throw redirect({ to: "/applications/$appId", params: { appId: "default" } })
  },
})
