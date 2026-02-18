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
    root: "group/sidebar relative flex h-full w-[var(--sidebar-width)] shrink-0 flex-col border-border border-r bg-background-elevated",
    header: "flex flex-col gap-4 border-border border-b px-4 pt-5 pb-4",
    body: "flex flex-1 flex-col gap-1 overflow-y-auto p-2",
    footer: "flex flex-col gap-2 border-border border-t px-3 pt-3 pb-4",
    item: "flex h-10 w-full items-center gap-2.5 rounded-lg border border-transparent px-3 font-medium text-sm transition-colors hover:bg-background-subtle",
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
    dragging: {
      true: {
        root: "transition-none",
      },
      false: {
        root: "transition-[width] duration-150 ease-out",
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
  defaultCollapsed?: boolean
  defaultWidth?: number
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
  defaultCollapsed = false,
  defaultWidth = 200,
  minWidth = 144,
  maxWidth = 360,
  compactThreshold = 112,
  compactWidth = 72,
  resizable = true,
  children,
  ...props
}: SidebarProps) {
  const [internalCompact, setInternalCompact] = useState(defaultCollapsed)
  const [internalWidth, setInternalWidth] = useState(() =>
    clampWidth(defaultWidth, minWidth, maxWidth),
  )
  const [isDragging, setIsDragging] = useState(false)
  const dragStateRef = useRef<{
    startX: number
    startWidth: number
  } | null>(null)
  const pointerIdRef = useRef<number | null>(null)
  const compactRef = useRef(internalCompact)
  const widthRef = useRef(internalWidth)
  const latestClientXRef = useRef<number | null>(null)
  const frameRequestRef = useRef<number | null>(null)

  const isCompact = internalCompact
  const expandedWidth = clampWidth(internalWidth, minWidth, maxWidth)
  const appliedWidth = isCompact ? compactWidth : expandedWidth

  useEffect(() => {
    compactRef.current = internalCompact
  }, [internalCompact])

  useEffect(() => {
    widthRef.current = internalWidth
  }, [internalWidth])

  const setCompact = useCallback((next: boolean) => {
    if (compactRef.current === next) {
      return
    }
    compactRef.current = next
    setInternalCompact(next)
  }, [])

  const setWidthValue = useCallback(
    (next: number) => {
      const clamped = clampWidth(next, minWidth, maxWidth)
      if (widthRef.current === clamped) {
        return
      }
      widthRef.current = clamped
      setInternalWidth(clamped)
    },
    [maxWidth, minWidth],
  )

  useEffect(() => {
    setWidthValue(widthRef.current)
  }, [setWidthValue])

  const processDrag = useCallback(
    (clientX: number) => {
      const dragState = dragStateRef.current
      if (!dragState) {
        return
      }

      const pointerWidth = dragState.startWidth + (clientX - dragState.startX)
      const clampedPointerWidth = clampWidth(
        pointerWidth,
        compactWidth,
        maxWidth,
      )

      if (clampedPointerWidth <= compactThreshold) {
        setCompact(true)
        return
      }

      setCompact(false)
      setWidthValue(clampedPointerWidth)
    },
    [compactThreshold, compactWidth, maxWidth, setCompact, setWidthValue],
  )

  const flushPendingFrame = useCallback(() => {
    if (frameRequestRef.current !== null) {
      cancelAnimationFrame(frameRequestRef.current)
      frameRequestRef.current = null
    }

    if (latestClientXRef.current === null) {
      return
    }

    const latestX = latestClientXRef.current
    latestClientXRef.current = null
    processDrag(latestX)
  }, [processDrag])

  const scheduleDrag = useCallback(
    (clientX: number) => {
      latestClientXRef.current = clientX
      if (frameRequestRef.current !== null) {
        return
      }

      frameRequestRef.current = requestAnimationFrame(() => {
        frameRequestRef.current = null
        if (latestClientXRef.current === null) {
          return
        }

        const latestX = latestClientXRef.current
        latestClientXRef.current = null
        processDrag(latestX)
      })
    },
    [processDrag],
  )

  const finishDrag = useCallback(() => {
    flushPendingFrame()
    setIsDragging(false)
    dragStateRef.current = null
    pointerIdRef.current = null
    document.body.style.userSelect = ""
  }, [flushPendingFrame])

  useEffect(() => {
    if (!isDragging) {
      return
    }

    const handlePointerMove = (event: PointerEvent) => {
      if (
        pointerIdRef.current !== null &&
        event.pointerId !== pointerIdRef.current
      ) {
        return
      }
      scheduleDrag(event.clientX)
    }

    const handlePointerUp = (event: PointerEvent) => {
      if (
        pointerIdRef.current !== null &&
        event.pointerId !== pointerIdRef.current
      ) {
        return
      }
      finishDrag()
    }

    window.addEventListener("pointermove", handlePointerMove)
    window.addEventListener("pointerup", handlePointerUp)
    window.addEventListener("pointercancel", handlePointerUp)

    return () => {
      flushPendingFrame()
      window.removeEventListener("pointermove", handlePointerMove)
      window.removeEventListener("pointerup", handlePointerUp)
      window.removeEventListener("pointercancel", handlePointerUp)
      document.body.style.userSelect = ""
    }
  }, [finishDrag, flushPendingFrame, isDragging, scheduleDrag])

  const handleRailPointerDown = (
    event: React.PointerEvent<HTMLButtonElement>,
  ) => {
    if (!resizable || event.button !== 0) return
    event.preventDefault()
    if (event.currentTarget.setPointerCapture) {
      try {
        event.currentTarget.setPointerCapture(event.pointerId)
      } catch {
        // Test environments may not track active pointers for synthetic events.
      }
    }
    pointerIdRef.current = event.pointerId
    setIsDragging(true)
    document.body.style.userSelect = "none"
    latestClientXRef.current = event.clientX
    dragStateRef.current = {
      startX: event.clientX,
      startWidth: compactRef.current ? compactWidth : widthRef.current,
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

  const { root, rail } = sidebar({ dragging: isDragging })
  const renderChildren =
    typeof children === "function"
      ? children({ compact: isCompact, width: appliedWidth })
      : children
  const contextValue = useMemo(() => ({ compact: isCompact }), [isCompact])

  return (
    <SidebarContext.Provider value={contextValue}>
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
