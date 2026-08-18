# Repository Guidelines

## Project Overview

Unreal Tools Lite is a Windows desktop application for inspecting and modifying Lineage 2 / Unreal Engine 2 texture assets. Its feature areas are UTX texture editing, batch UTX extraction, batch texture resizing, and geodata conversion. The stack is Tauri 2 with a React 19/TypeScript renderer and a Rust backend.

## Architecture & Data Flow

- `src/main.tsx` mounts the single `App` component. `src/App.tsx` currently owns the renderer UI, feature state, dialogs, persistence, and Tauri calls; there is no router, global store, service layer, or generated IPC client.
- Frontend handlers call snake_case commands with `invoke(...)` from `@tauri-apps/api/core`. Argument objects and response DTOs use camelCase. Keep TypeScript DTOs aligned with Rust structs using `#[serde(rename_all = "camelCase")]`.
- `src-tauri/src/main.rs` delegates to `unreal_tools_lib::run`. `src-tauri/src/lib.rs` is the composition and transport boundary: it registers plugins, managed state, and thin `#[tauri::command]` wrappers.
- Keep Tauri types in `lib.rs`. Put parsing, validation, filesystem work, and format-specific business logic in the Tauri-independent domain modules: `utx.rs`, `texture_engine.rs`, `texture_resize.rs`, and `geodata.rs`.
- Rust commands are synchronous. Renderer handlers are async request/response flows using `try/catch/finally`; there is no custom event bus.
- Expensive session data uses Tauri-managed state: `UtxCache` is a `Mutex<Option<_>>` value bound to the selected file. Other UI state stays in React, and recent paths/preferences use `window.localStorage` keys under `unreal-tools.*`.
- Typical flow: UI handler → Tauri `invoke` → thin command in `lib.rs` → domain module parses/validates/updates data → serde DTO/result → React state/toast. Preview commands return base64 PNG data URLs.
- Destructive operations require renderer confirmation, but backend modules must still validate paths, formats, dimensions, and package metadata. Preserve original encryption and XML text encoding when rewriting files.

## UTX Template Contract

- `src-tauri/assets/UnrealTlp.utx` is embedded with `include_bytes!` and is the editor-generated structural template for creating and importing textures into UTX files. Keep it a valid, unencrypted Unreal Engine 2 package; do not replace it with a file merely renamed to `.utx`.
- Creating a new UTX retains the template's name/import structure, renames the package to the destination filename, and removes its exports. The template's seed textures must therefore never appear in a newly created package.
- The template contains `Common`, `TlpSpt9`, `TlpAnim`, and `TlpSplitAnim` as editor-generated structural references. The importer uses a normal/Split9/animation seed according to the requested metadata; Split9 + animation does **not** depend on `TlpSplitAnim`, because the engine layers Split9 properties onto an animation seed. None of these exports may appear in a newly created package.
- `TlpAnim` must be saved by Unreal Editor with every supported animation property serialized: `AnimNext`, `MaxFrameRate`, `MinFrameRate`, `OneTimeAnimLoop`, `PrimeCount`, and `TotalFrameNum`. Unreal omits default values, so set `OneTimeAnimLoop=true` and `PrimeCount` to a value greater than zero before saving the template; the native engine can then safely set them back to `false` and `0` when needed.
- `texture_engine.rs` owns every mutating UTX flow: clean creation, texture add, batch import, and replacement. `utx.rs` is the reader/export/UI-cache bridge. Do not reintroduce writers in `utx.rs`.
- The companion texture metadata file is sectioned: `[Texture]` optionally carries `Alpha`, `Masked`, `UClamp`, `VClamp`, `UClampMode`, and `VClampMode`; `[Split9]` carries the six borders; `[Animations]` carries the animation fields. Omitted settings preserve their target/seed value.
- The native writer parses UE2 property streams, preserves every unknown property byte, and changes only requested known fields. It can add the canonical properties needed for alpha, masked, clamp, Split9, and animation when they are missing. It must preserve the `None` terminator and all post-property texture data exactly; otherwise Unreal reports `Serial size mismatch`.

## Key Directories

- `src/`: React renderer. `App.tsx` contains the current feature implementation; `App.css` is the shared stylesheet; `assets/` contains bundled assets.
- `src-tauri/src/`: Rust entry points, Tauri command boundary, domain logic, and colocated unit tests.
- `src-tauri/capabilities/`: Tauri permissions for the `main` window.
- `public/`: static files copied by Vite.
- `dist/`, `node_modules/`, `src-tauri/target/`, and `src-tauri/gen/schemas/`: generated or dependency output. Do not hand-edit or commit these paths.

## Development Commands

Run commands from the repository root unless noted.

```bash
pnpm install                 # install frontend/Tauri CLI dependencies
pnpm dev                     # frontend-only Vite server on port 1420
pnpm tauri dev               # full desktop development application
pnpm build                   # TypeScript check, then Vite production build
pnpm preview                 # preview the built frontend only
pnpm tauri build             # frontend + Rust build and Windows NSIS package
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

There are no declared frontend `test`, `lint`, or `format` scripts. If Rust formatting or linting is specifically needed, use conventional Cargo commands:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

## Code Conventions & Common Patterns

- TypeScript is strict and rejects unused locals/parameters and switch fallthrough. Use PascalCase for components/types, camelCase for helpers/state, and SCREAMING_SNAKE_CASE for constants.
- Rust uses snake_case functions/modules and SCREAMING_SNAKE_CASE constants. Domain modules define local `Result<T, String>` aliases and add Portuguese, user-facing context with `map_err`, `ok_or`, and `?`. Avoid production `unwrap()`.
- Keep IPC wrappers thin. Do not introduce Tauri dependencies into format parsers or duplicate domain validation in the command layer.
- Follow existing React patterns: local `useState`, derived collections in `useMemo`, effects for integration behavior, and `useRef` request counters where stale async preview/search responses could overwrite newer state.
- Frontend command handlers derive displayable errors through the existing `errorText` helper, set/reset shared busy state, and show Portuguese toast messages.
- Binary parsers use checked offsets/arithmetic, bounded slice access, private reader/package records, and separate public wire DTOs. Preallocate from validated counts rather than repeatedly growing collections.
- Prefer deterministic output (`BTreeMap`/`BTreeSet`, explicit sorting) for indexed XML and generated mappings. Batch operations continue past item failures and return success/skipped/failed summaries.
- Preserve existing safety bounds: UTX import/extract cap reported errors at 20; texture resize and geodata conversion cap reported errors at 30.
- When adding or changing an IPC DTO or command, update the Rust command registration in `lib.rs`, its serde shape, every `invoke` call, and the manually mirrored TypeScript type together.

## Important Files

- `src/main.tsx`: renderer bootstrap.
- `src/App.tsx`: all current UI features, Tauri invokes, dialogs, local state, and persistence.
- `src-tauri/src/main.rs`: native binary entry point.
- `src-tauri/src/lib.rs`: Tauri app builder, state injection, command wrappers, and command registry.
- `src-tauri/src/utx.rs`: UTX reading, preview/export, encryption handling, metadata parsing, UI cache, and synthesized-fixture tests.
- `src-tauri/src/texture_engine.rs`: source of truth for native UE2 UTX creation, add/import/replace, property editing, table serialization, and GUI-ready texture editor DTOs. Preserve unknown property bytes unless the user changes that property.
- `src-tauri/assets/UnrealTlp.utx`: embedded UE2 UTX template and structural seeds used by UTX creation/import.
- `src-tauri/src/texture_resize.rs`: batch DDS/TGA resizing for Unreal Engine 2 texture resolutions.
- `src-tauri/src/geodata.rs`: batch geodata conversion between L2J, CONV_DAT, and L2G formats.
- `package.json`: authoritative pnpm scripts and frontend dependencies.
- `src-tauri/Cargo.toml`: Rust dependencies, crate outputs, and release profile.
- `src-tauri/tauri.conf.json`: Vite/Tauri lifecycle, window settings, and NSIS packaging.
- `vite.config.ts`: fixed development/HMR ports and Tauri-aware watcher settings.
- `tsconfig.json`: strict frontend compiler policy.
- `src-tauri/capabilities/default.json`: desktop permission boundary.
- `README.md`: Portuguese user/developer overview and release artifact locations.

## Runtime/Tooling Preferences

- Use pnpm; the repository commits `pnpm-lock.yaml`, and Tauri lifecycle hooks call pnpm directly. Do not substitute npm or yarn.
- Use Node `^20.19.0` or `>=22.12.0`, matching the locked Vite 8 toolchain. No exact pnpm release is pinned.
- Use stable Rust/Cargo with Rust 2021 edition. No `rust-toolchain` file or minimum Rust version is pinned.
- Windows is the evidenced build/package target. Development requires the normal Tauri Windows prerequisites, including Visual Studio C++ build tools; production packaging targets NSIS only.
- Vite uses strict port `1420`; `TAURI_DEV_HOST` enables network hosting with HMR on `1421`. An occupied port fails startup rather than selecting another port.
- Maintain both lockfiles when dependencies change: `pnpm-lock.yaml` and `src-tauri/Cargo.lock`.

## Testing & QA

- Required repository checks documented by the project are:

  ```bash
  pnpm build
  cargo test --manifest-path src-tauri/Cargo.toml
  ```

- Rust tests are synchronous inline `#[cfg(test)] mod tests` modules beside each domain implementation. Use descriptive snake_case names and standard `#[test]`/`assert!`/`assert_eq!` patterns.
- Build binary-format fixtures in memory, following the `fixture_package`/`sample_tga` patterns in `utx.rs`. Filesystem tests create unique directories under `std::env::temp_dir()` and remove them after success.
- Add tests for observable parser/workflow behavior, boundaries, invalid inputs, and write invariants. Domain tests should call Tauri-independent functions directly.
- There is no frontend test runner, integration/E2E harness, CI workflow, coverage tool, or coverage threshold. `pnpm build` is the current frontend typecheck/bundle gate, not a test suite.
