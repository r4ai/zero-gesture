# Zero Gesture

A lightweight mouse gesture app for Windows.

<video src="./.github/assets/demo.mp4" autoplay loop muted></video>

## Development

### Prerequisites

- Windows 11
- Node.js (>= 24)
- pnpm (>= 10)
- Rust (stable)

### Quick Start

1. Clone the repository:

   ```sh
   git clone https://github.com/r4ai/zero-gesture.git
   cd zero-gesture
   ```

2. Install dependencies:

   ```sh
   pnpm install
   ```

3. Start the development server:

   ```sh
   pnpm tauri dev
   ```

4. Build the application for production:

   ```sh
   pnpm tauri build
   ```

### Docs

- [Architecture](./docs/architecture.md)
- [Development Guide](./docs/development.md)

### Environment Variables

- `ZG_LOG_LEVEL`: Set the log level for the application.
  - Valid values: `error`, `warn`, `info`, `debug`, `trace`
  - Default: `info`
