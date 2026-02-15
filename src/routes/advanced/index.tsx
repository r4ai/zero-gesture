import { createFileRoute } from "@tanstack/react-router"
import { Download, FolderOpen, Upload } from "lucide-react"
import { Button } from "@/components/ui/button"

function AdvancedPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-bold text-2xl tracking-tight">Advanced</h2>
        <p className="text-muted-foreground">
          Advanced configuration options for power users.
        </p>
      </div>

      <div className="space-y-4">
        {/* Import Config */}
        <div className="flex items-center justify-between rounded-lg border px-6 py-5">
          <div className="space-y-1">
            <p className="font-semibold leading-none">Import Config</p>
            <p className="text-muted-foreground text-sm">
              Load a JSON config file and apply it as the current configuration.
            </p>
          </div>
          <Button variant="outline" className="ml-6 shrink-0">
            <Upload className="mr-2 h-4 w-4" />
            Import
          </Button>
        </div>

        {/* Export Config */}
        <div className="flex items-center justify-between rounded-lg border px-6 py-5">
          <div className="space-y-1">
            <p className="font-semibold leading-none">Export Config</p>
            <p className="text-muted-foreground text-sm">
              Save the current configuration to a JSON file.
            </p>
          </div>
          <Button variant="outline" className="ml-6 shrink-0">
            <Download className="mr-2 h-4 w-4" />
            Export
          </Button>
        </div>

        {/* Config File Location */}
        <div className="flex items-center justify-between rounded-lg border px-6 py-5">
          <div className="space-y-1">
            <p className="font-semibold leading-none">Config File Location</p>
            <p className="text-muted-foreground text-sm">
              Open the folder where the config file is stored.
            </p>
          </div>
          <Button variant="outline" className="ml-6 shrink-0">
            <FolderOpen className="mr-2 h-4 w-4" />
            Open Folder
          </Button>
        </div>
      </div>
    </div>
  )
}

export const Route = createFileRoute("/advanced/")({
  component: AdvancedPage,
})
