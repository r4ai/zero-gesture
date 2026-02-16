import {
  type CSSProperties,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { tv } from "tailwind-variants"

const sidebar = tv({
  slots: {
    root: "group/sidebar relative flex h-full w-[var(--sidebar-width)] shrink-0 flex-col border-border border-r bg-background-elevated transition-[width] duration-150 ease-out",
    header: "flex flex-col gap-4 border-border border-b px-4 pt-5 pb-4",
    body: "flex flex-1 flex-col gap-1 overflow-y-auto p-2",
    footer: "flex flex-col gap-2 border-border border-t px-3 pt-3 pb-4",
    item: "flex h-10 w-full items-center gap-2.5 rounded-lg border border-transparent px-3 font-medium text-[13px] transition-colors hover:bg-background-subtle",
    rail: "absolute top-0 -right-1 z-20 h-full w-2 cursor-col-resize touch-none bg-transparent after:absolute after:top-0 after:left-1/2 after:h-full after:w-px after:-translate-x-1/2 after:bg-transparent hover:after:bg-border data-[dragging=true]:after:bg-border-bright",
  },
  variants: {
    compact: {
      true: {
        header: "items-center gap-3 px-0 pt-[18px] pb-[14px]",
        body: "items-center gap-2 px-0 py-[10px]",
        footer: "items-center gap-2 px-0 pt-[10px] pb-[14px]",
        item: "h-11 w-11 justify-center gap-0 rounded-[10px] px-0",
      },
    },
    active: {
      true: {
        item: "border-border-bright bg-background-card font-semibold text-foreground",
      },
      false: {
        item: "text-foreground-muted",
      },
    },
  },
})

interface SidebarRenderState {
  compact: boolean
  width: number
}

interface SidebarContextValue {
  compact: boolean
}

const SidebarContext = createContext<SidebarContextValue>({
  compact: false,
})

function useSidebarContext() {
  return useContext(SidebarContext)
}

type SidebarProps = Omit<React.HTMLAttributes<HTMLDivElement>, "children"> & {
  collapsed?: boolean
  defaultCollapsed?: boolean
  onCollapsedChange?: (collapsed: boolean) => void
  width?: number
  defaultWidth?: number
  onWidthChange?: (width: number) => void
  minWidth?: number
  maxWidth?: number
  compactThreshold?: number
  compactWidth?: number
  resizable?: boolean
  children?: React.ReactNode | ((state: SidebarRenderState) => React.ReactNode)
}

function clampWidth(width: number, min: number, max: number) {
  return Math.min(max, Math.max(min, width))
}

export function Sidebar({
  className,
  style,
  collapsed,
  defaultCollapsed = false,
  onCollapsedChange,
  width,
  defaultWidth = 200,
  onWidthChange,
  minWidth = 144,
  maxWidth = 360,
  compactThreshold = 112,
  compactWidth = 72,
  resizable = true,
  children,
  ...props
}: SidebarProps) {
  const [internalCompact, setInternalCompact] = useState(defaultCollapsed)
  const [internalWidth, setInternalWidth] = useState(defaultWidth)
  const [isDragging, setIsDragging] = useState(false)
  const dragStateRef = useRef<{
    startX: number
    startWidth: number
  } | null>(null)

  const isCompact = collapsed ?? internalCompact
  const currentWidth = width ?? internalWidth
  const expandedWidth = clampWidth(currentWidth, minWidth, maxWidth)
  const appliedWidth = isCompact ? compactWidth : expandedWidth

  const setCompact = useCallback(
    (next: boolean) => {
      if (collapsed !== undefined) {
        onCollapsedChange?.(next)
        return
      }
      setInternalCompact(next)
      onCollapsedChange?.(next)
    },
    [collapsed, onCollapsedChange],
  )

  const setWidthValue = useCallback(
    (next: number) => {
      const clamped = clampWidth(next, minWidth, maxWidth)
      if (width === undefined) {
        setInternalWidth(clamped)
      }
      onWidthChange?.(clamped)
    },
    [maxWidth, minWidth, onWidthChange, width],
  )

  useEffect(() => {
    if (!isDragging) return

    const handlePointerMove = (event: PointerEvent) => {
      const dragState = dragStateRef.current
      if (!dragState) return

      if (isCompact) {
        const compactDragDelta = event.clientX - dragState.startX
        if (compactDragDelta <= 0) {
          setCompact(true)
          return
        }

        const compactDragWidth = compactWidth + compactDragDelta
        if (compactDragWidth <= compactThreshold) {
          setCompact(true)
          return
        }

        setCompact(false)
        setWidthValue(clampWidth(compactDragWidth, minWidth, maxWidth))
        return
      }

      const pointerWidth =
        dragState.startWidth + (event.clientX - dragState.startX)
      const clampedPointerWidth = clampWidth(
        pointerWidth,
        compactWidth,
        maxWidth,
      )

      if (clampedPointerWidth <= compactThreshold) {
        setCompact(true)
        return
      }

      setWidthValue(clampWidth(clampedPointerWidth, minWidth, maxWidth))
    }

    const handlePointerUp = () => {
      setIsDragging(false)
      dragStateRef.current = null
      document.body.style.userSelect = ""
    }

    window.addEventListener("pointermove", handlePointerMove)
    window.addEventListener("pointerup", handlePointerUp)
    window.addEventListener("pointercancel", handlePointerUp)

    return () => {
      window.removeEventListener("pointermove", handlePointerMove)
      window.removeEventListener("pointerup", handlePointerUp)
      window.removeEventListener("pointercancel", handlePointerUp)
      document.body.style.userSelect = ""
    }
  }, [
    compactThreshold,
    compactWidth,
    isCompact,
    isDragging,
    maxWidth,
    minWidth,
    setCompact,
    setWidthValue,
  ])

  const handleRailPointerDown = (
    event: React.PointerEvent<HTMLButtonElement>,
  ) => {
    if (!resizable || event.button !== 0) return
    event.preventDefault()
    setIsDragging(true)
    document.body.style.userSelect = "none"
    dragStateRef.current = {
      startX: event.clientX,
      startWidth: isCompact ? compactWidth : expandedWidth,
    }
  }

  const inlineStyle = useMemo(
    () =>
      ({
        ...style,
        "--sidebar-width": `${appliedWidth}px`,
      }) as CSSProperties,
    [appliedWidth, style],
  )

  const { root, rail } = sidebar()
  const renderChildren =
    typeof children === "function"
      ? children({ compact: isCompact, width: appliedWidth })
      : children

  return (
    <SidebarContext.Provider value={{ compact: isCompact }}>
      <div
        data-compact={isCompact}
        className={root({ className })}
        style={inlineStyle}
        {...props}
      >
        {renderChildren}
        {resizable ? (
          <button
            type="button"
            aria-label="Resize sidebar"
            data-dragging={isDragging}
            className={rail()}
            onPointerDown={handleRailPointerDown}
          />
        ) : null}
      </div>
    </SidebarContext.Provider>
  )
}

export function SidebarHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { compact } = useSidebarContext()
  const { header } = sidebar({ compact })
  return (
    <div data-compact={compact} className={header({ className })} {...props} />
  )
}

export function SidebarBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { compact } = useSidebarContext()
  const { body } = sidebar({ compact })
  return (
    <div data-compact={compact} className={body({ className })} {...props} />
  )
}

export function SidebarFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { compact } = useSidebarContext()
  const { footer } = sidebar({ compact })
  return (
    <div data-compact={compact} className={footer({ className })} {...props} />
  )
}

export function SidebarItem({
  className,
  active,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { active?: boolean }) {
  const { compact } = useSidebarContext()
  const { item } = sidebar({ active, compact })
  return (
    <button
      type="button"
      data-compact={compact}
      className={item({ className })}
      {...props}
    />
  )
}
