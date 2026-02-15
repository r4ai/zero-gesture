import React from "react"

export interface UseSidebarResizeProps {
  direction?: "left" | "right"
  currentWidth: string
  onResize: (width: string) => void
  onToggle?: () => void
  isCollapsed?: boolean
  minResizeWidth?: string
  maxResizeWidth?: string
  enableAutoCollapse?: boolean
  autoCollapseThreshold?: number
  expandThreshold?: number
  enableDrag?: boolean
  setIsDraggingRail?: (isDragging: boolean) => void
  widthCookieName?: string
  widthCookieMaxAge?: number
}

interface WidthUnit {
  value: number
  unit: "rem" | "px"
}

function parseWidth(width: string): WidthUnit {
  const unit = width.endsWith("rem") ? "rem" : "px"
  const value = Number.parseFloat(width)
  return { value, unit }
}

function toPx(width: string): number {
  const { value, unit } = parseWidth(width)
  return unit === "rem" ? value * 16 : value
}

function formatWidth(value: number, unit: "rem" | "px"): string {
  return `${unit === "rem" ? value.toFixed(1) : Math.round(value)}${unit}`
}

export function useSidebarResize({
  direction = "right",
  currentWidth,
  onResize,
  onToggle,
  isCollapsed = false,
  minResizeWidth = "14rem",
  maxResizeWidth = "24rem",
  enableAutoCollapse = true,
  autoCollapseThreshold = 1.5,
  expandThreshold = 0.2,
  enableDrag = true,
  setIsDraggingRail = () => {},
  widthCookieName,
  widthCookieMaxAge = 60 * 60 * 24 * 7,
}: UseSidebarResizeProps) {
  const dragRef = React.useRef<HTMLButtonElement>(null)
  const startWidth = React.useRef(0)
  const startX = React.useRef(0)
  const isDragging = React.useRef(false)
  const isInteractingWithRail = React.useRef(false)
  const lastTogglePoint = React.useRef(0)
  const toggleCooldown = React.useRef(false)
  const lastToggleTime = React.useRef(0)

  const minWidthPx = React.useMemo(() => toPx(minResizeWidth), [minResizeWidth])
  const maxWidthPx = React.useMemo(() => toPx(maxResizeWidth), [maxResizeWidth])

  const persistWidth = React.useCallback(
    (width: string) => {
      if (widthCookieName) {
        // biome-ignore lint/suspicious/noDocumentCookie: this is fine
        document.cookie = `${widthCookieName}=${width}; path=/; max-age=${widthCookieMaxAge}`
      }
    },
    [widthCookieName, widthCookieMaxAge],
  )

  const handleMouseDown = React.useCallback(
    (e: React.MouseEvent) => {
      isInteractingWithRail.current = true

      if (!enableDrag) return

      const currentWidthPx = isCollapsed ? 0 : toPx(currentWidth)
      startWidth.current = currentWidthPx
      startX.current = e.clientX
      lastTogglePoint.current = e.clientX
      toggleCooldown.current = false
      lastToggleTime.current = 0

      e.preventDefault()
    },
    [enableDrag, isCollapsed, currentWidth],
  )

  React.useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isInteractingWithRail.current) return

      const deltaX = Math.abs(e.clientX - startX.current)
      if (!isDragging.current && deltaX > 5) {
        isDragging.current = true
        setIsDraggingRail(true)
      }

      if (isDragging.current) {
        const { unit } = parseWidth(currentWidth)
        const currentDragDirection =
          direction === "left"
            ? e.clientX < lastTogglePoint.current
              ? "expand"
              : "collapse"
            : e.clientX > lastTogglePoint.current
              ? "expand"
              : "collapse"

        const dragDistanceFromToggle = Math.abs(
          e.clientX - lastTogglePoint.current,
        )
        const now = Date.now()

        if (toggleCooldown.current && now - lastToggleTime.current > 200) {
          toggleCooldown.current = false
        }

        if (!toggleCooldown.current) {
          if (enableAutoCollapse && onToggle && !isCollapsed) {
            let shouldCollapse = false

            if (autoCollapseThreshold <= 1.0) {
              const currentDragWidth =
                direction === "left" ? window.innerWidth - e.clientX : e.clientX
              shouldCollapse =
                currentDragWidth <= minWidthPx * autoCollapseThreshold
            } else {
              const currentDragWidth =
                direction === "left" ? window.innerWidth - e.clientX : e.clientX
              if (currentDragWidth <= minWidthPx) {
                const extraDragNeeded =
                  minWidthPx * (autoCollapseThreshold - 1.0)
                const distanceBeyondMin = minWidthPx - currentDragWidth
                shouldCollapse = distanceBeyondMin >= extraDragNeeded
              }
            }

            if (currentDragDirection === "collapse" && shouldCollapse) {
              onToggle()
              lastTogglePoint.current = e.clientX
              toggleCooldown.current = true
              lastToggleTime.current = now
              return
            }
          }

          if (
            onToggle &&
            isCollapsed &&
            currentDragDirection === "expand" &&
            dragDistanceFromToggle > minWidthPx * expandThreshold
          ) {
            onToggle()

            const initialWidth =
              direction === "left" ? window.innerWidth - e.clientX : e.clientX
            const clampedWidth = Math.max(
              minWidthPx,
              Math.min(maxWidthPx, initialWidth),
            )
            const formattedWidth = formatWidth(
              unit === "rem" ? clampedWidth / 16 : clampedWidth,
              unit,
            )
            onResize(formattedWidth)
            persistWidth(formattedWidth)

            lastTogglePoint.current = e.clientX
            toggleCooldown.current = true
            lastToggleTime.current = now
            return
          }
        }

        if (isCollapsed) return

        const newWidthPx =
          direction === "left" ? window.innerWidth - e.clientX : e.clientX
        const clampedWidthPx = Math.max(
          minWidthPx,
          Math.min(maxWidthPx, newWidthPx),
        )
        const newWidth = unit === "rem" ? clampedWidthPx / 16 : clampedWidthPx
        const formattedWidth = formatWidth(newWidth, unit)

        onResize(formattedWidth)
        persistWidth(formattedWidth)
      }
    }

    const handleMouseUp = () => {
      if (!isInteractingWithRail.current) return

      if (!isDragging.current && onToggle) {
        onToggle()
      }

      isDragging.current = false
      isInteractingWithRail.current = false
      lastTogglePoint.current = 0
      toggleCooldown.current = false
      setIsDraggingRail(false)
    }

    document.addEventListener("mousemove", handleMouseMove)
    document.addEventListener("mouseup", handleMouseUp)

    return () => {
      document.removeEventListener("mousemove", handleMouseMove)
      document.removeEventListener("mouseup", handleMouseUp)
    }
  }, [
    onResize,
    onToggle,
    isCollapsed,
    currentWidth,
    persistWidth,
    setIsDraggingRail,
    minWidthPx,
    maxWidthPx,
    direction,
    enableAutoCollapse,
    autoCollapseThreshold,
    expandThreshold,
  ])

  return {
    dragRef,
    isDragging,
    handleMouseDown,
  }
}
