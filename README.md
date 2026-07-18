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

- [Architecture](./docs/architecture.md)
- [Development Guide](./docs/development.md)
- [Release Guide](./docs/releasing.md)

## Alpha releases

Windows x64 alpha installers are distributed from the repository's
[GitHub Releases](https://github.com/r4ai/zero-gesture/releases) page. Download
the NSIS `.exe` for typical installations, or the `.msi` for managed Windows
deployments. Verify the downloaded file against the accompanying
`SHA256SUMS.txt` before installation.

The installers are currently unsigned, so Windows SmartScreen may show a
warning. Only install release assets published from this repository.

### Environment Variables

- `ZG_LOG_LEVEL`: Set the log level for the application.
  - Valid values: `error`, `warn`, `info`, `debug`, `trace`
  - Default: `info`
