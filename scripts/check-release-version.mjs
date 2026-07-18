import { readFile } from "node:fs/promises"
import { resolve } from "node:path"

const root = resolve(import.meta.dirname, "..")
const alphaVersionPattern = /^0\.\d+\.\d+-alpha\.\d+$/

async function readJson(relativePath) {
  return JSON.parse(await readFile(resolve(root, relativePath), "utf8"))
}

function readCargoPackageVersion(cargoToml) {
  const packageSection = cargoToml.match(/^\[package\]\r?\n([\s\S]*?)(?=^\[)/m)
  const version = packageSection?.[1].match(
    /^version\s*=\s*"([^"]+)"\s*$/m,
  )?.[1]

  if (!version) {
    throw new Error(
      "Unable to read [package].version from src-tauri/Cargo.toml.",
    )
  }

  return version
}

function readCargoLockPackageVersion(cargoLock) {
  const packageEntry = cargoLock.match(
    /\[\[package\]\]\r?\nname\s*=\s*"zero-gesture"\r?\nversion\s*=\s*"([^"]+)"/,
  )

  if (!packageEntry) {
    throw new Error(
      "Unable to read zero-gesture version from src-tauri/Cargo.lock.",
    )
  }

  return packageEntry[1]
}

const [packageJson, tauriConfig, cargoToml, cargoLock] = await Promise.all([
  readJson("package.json"),
  readJson("src-tauri/tauri.conf.json"),
  readFile(resolve(root, "src-tauri/Cargo.toml"), "utf8"),
  readFile(resolve(root, "src-tauri/Cargo.lock"), "utf8"),
])

const versions = {
  "package.json": packageJson.version,
  "src-tauri/tauri.conf.json": tauriConfig.version,
  "src-tauri/Cargo.toml": readCargoPackageVersion(cargoToml),
  "src-tauri/Cargo.lock": readCargoLockPackageVersion(cargoLock),
}
const uniqueVersions = [...new Set(Object.values(versions))]

if (uniqueVersions.length !== 1) {
  throw new Error(
    `Release versions must match:\n${Object.entries(versions)
      .map(([file, version]) => `  ${file}: ${version}`)
      .join("\n")}`,
  )
}

const [version] = uniqueVersions
if (!alphaVersionPattern.test(version)) {
  throw new Error(
    `Expected an alpha version (0.x.y-alpha.N), received ${version}.`,
  )
}

const tag = process.env.RELEASE_TAG
if (tag && tag !== `v${version}`) {
  throw new Error(
    `Release tag ${tag} does not match application version v${version}.`,
  )
}

console.log(`Release version is synchronized: ${version}`)
