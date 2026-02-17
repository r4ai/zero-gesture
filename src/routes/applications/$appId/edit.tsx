import {
  createFileRoute,
  Link,
  useNavigate,
  useParams,
} from "@tanstack/react-router"
import { ArrowLeft, Globe, Trash2 } from "lucide-react"
import { APPS } from "@/components/applications/app-settings-layout"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Select, SelectItem } from "@/components/ui/select"

export const Route = createFileRoute("/applications/$appId/edit")({
  component: AppEditPage,
})

// Mock matching conditions
const mockConditions = [
  { id: 1, field: "process_name", method: "exact", value: "google-chrome" },
  { id: 2, field: "window_class", method: "contains", value: "chrome" },
]

/**
 * App Edit Page - Edit application matching conditions
 * Based on Pencil: "Applications Settings - App Edit"
 */
function AppEditPage() {
  const { appId } = useParams({ from: "/applications/$appId/edit" })
  const navigate = useNavigate()

  const currentApp = APPS.find((app) => app.id === appId)

  if (!currentApp) {
    navigate({ to: "/applications" })
    return null
  }

  return (
    <div className="flex h-full flex-1 flex-col bg-background">
      {/* Header */}
      <div className="flex h-16 items-center justify-between border-border border-b px-6">
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 items-center justify-center rounded-md bg-background-subtle">
            <Globe className="h-4 w-4 text-foreground-subtle" />
          </div>
          <div className="flex flex-col">
            <h2 className="font-semibold text-[16px] text-foreground">
              {currentApp.name}
            </h2>
            <span className="text-[12px] text-foreground-subtle">
              {mockConditions.length} matching conditions
            </span>
          </div>
        </div>
        <Button
          variant="outline"
          className="h-8 gap-2 rounded-md border-destructive-subtle bg-destructive-subtle text-[12px] text-destructive hover:bg-destructive/20"
        >
          <Trash2 className="h-3.5 w-3.5" />
          <span>Delete App</span>
        </Button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-6">
        {/* Back Button */}
        <Link
          to="/applications/$appId"
          params={{ appId }}
          className="mb-6 inline-flex h-8 items-center gap-2 rounded-md border border-border-muted bg-transparent px-3 text-[12px] text-foreground-muted transition-colors hover:text-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          <span>Back to Gesture Edit</span>
        </Link>

        {/* Section Header */}
        <div className="mb-6 flex items-center justify-between">
          <div className="flex flex-col gap-1">
            <h3 className="font-semibold text-[15px] text-foreground">
              Matching Conditions
            </h3>
            <p className="text-[12px] text-foreground-subtle">
              App matches when ANY condition is met
            </p>
          </div>
        </div>

        {/* Conditions List */}
        <div className="flex flex-col gap-3">
          {mockConditions.map((condition, index) => (
            <div
              key={condition.id}
              className="flex flex-col gap-3 rounded-lg border border-border bg-background-elevated p-4"
            >
              <div className="flex items-center justify-between">
                <Badge variant="success" className="text-[11px]">
                  Condition {index + 1}
                </Badge>
                <button
                  type="button"
                  className="text-foreground-muted hover:text-foreground"
                >
                  ×
                </button>
              </div>
              <div className="flex items-center gap-2">
                <Select
                  value={condition.field}
                  onChange={() => {}}
                  className="flex-1"
                >
                  <SelectItem id="process_name" textValue="Process Name">
                    Process Name
                  </SelectItem>
                  <SelectItem id="window_class" textValue="Window Class">
                    Window Class
                  </SelectItem>
                  <SelectItem id="window_title" textValue="Window Title">
                    Window Title
                  </SelectItem>
                </Select>
                <Select
                  value={condition.method}
                  onChange={() => {}}
                  className="w-28"
                >
                  <SelectItem id="exact" textValue="exact">
                    exact
                  </SelectItem>
                  <SelectItem id="contains" textValue="contains">
                    contains
                  </SelectItem>
                  <SelectItem id="regex" textValue="regex">
                    regex
                  </SelectItem>
                </Select>
              </div>
              <input
                type="text"
                value={condition.value}
                className="h-9 rounded-md border border-border bg-background-card px-3 text-[13px] text-foreground"
                readOnly
              />
            </div>
          ))}
        </div>
      </div>

      {/* Footer */}
      <div className="flex h-16 items-center justify-end gap-3 border-border border-t px-6">
        <Link to="/applications/$appId" params={{ appId }}>
          <Button variant="outline" className="h-9 px-4 text-[13px]">
            Cancel
          </Button>
        </Link>
        <Button className="h-9 gap-2 px-4 text-[13px]">
          <span>Save Changes</span>
        </Button>
      </div>
    </div>
  )
}
