import { createFileRoute } from "@tanstack/react-router"

function GeneralPage() {
  return <div></div>
}

export const Route = createFileRoute("/general/")({
  component: GeneralPage,
})
