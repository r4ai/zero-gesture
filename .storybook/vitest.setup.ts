// Replace your-framework with the framework you are using, e.g. react-vite, nextjs, nextjs-vite, etc.
import { setProjectAnnotations } from "@storybook/react"
import * as previewAnnotations from "./preview"

setProjectAnnotations([previewAnnotations])
