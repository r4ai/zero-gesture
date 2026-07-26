# Zero Gesture

A lightweight mouse gesture app for Windows.

<video src="https://github.com/user-attachments/assets/3cbf84e8-ad2b-4d6a-ab58-17797bab0463" widht="100%" autoplay loop muted></video>

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

- [Documentation index](./docs/README.md)
- [Architecture](./docs/architecture.md)
- [Architecture decisions](./docs/adr/README.md)
- [Development Guide](./docs/development.md)

### Environment Variables

- `ZG_LOG_LEVEL`: Set the log level for the application.
  - Valid values: `error`, `warn`, `info`, `debug`, `trace`
  - Default: `info`
