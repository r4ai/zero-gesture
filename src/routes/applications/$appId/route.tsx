import { createFileRoute, Outlet, useParams } from "@tanstack/react-router"
import { AppPanelLayout } from "@/components/applications/app-settings-layout"

export const Route = createFileRoute("/applications/$appId")({
  component: ApplicationsAppLayout,
})

function ApplicationsAppLayout() {
  const { appId } = useParams({ from: "/applications/$appId" })

  return (
    <AppPanelLayout appId={appId}>
      <Outlet />
    </AppPanelLayout>
  )
}
