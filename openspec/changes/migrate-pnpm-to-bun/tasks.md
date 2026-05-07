## 1. Package manifest updates

- [x] 1.1 Update root `package.json`: change `"packageManager"` to `"bun@1.3.13"` and add `"workspaces": ["web", "third_party/vscode-codicons"]`
- [x] 1.2 Update `web/package.json`: change `"packageManager"` to `"bun@1.3.13"`
- [x] 1.3 Add `bunfig.toml` at repo root with `[install]\nsave-text-lockfile = true`

## 2. Lockfile migration

- [x] 2.1 Delete `pnpm-workspace.yaml`
- [x] 2.2 Delete `pnpm-lock.yaml`
- [x] 2.3 Run `bun install` at repo root to generate `bun.lock`
- [x] 2.4 Verify `bun.lock` is committed and `pnpm-lock.yaml` / `pnpm-workspace.yaml` are removed from git

## 3. CMake integration

- [x] 3.1 In `cmake/CronymaxApp.cmake`: replace `find_program(PNPM_EXECUTABLE pnpm)` with `find_program(BUN_EXECUTABLE bun)`
- [x] 3.2 Update the `FATAL_ERROR` message to reference bun instead of pnpm
- [x] 3.3 Replace `${PNPM_EXECUTABLE} install --frozen-lockfile` with `${BUN_EXECUTABLE} install --frozen-lockfile` in `cronymax_web` target
- [x] 3.4 Replace `${PNPM_EXECUTABLE} build` with `${BUN_EXECUTABLE} run build` in `cronymax_web` target
- [x] 3.5 Replace `${PNPM_EXECUTABLE} typecheck` and `${PNPM_EXECUTABLE} lint` with `${BUN_EXECUTABLE} run typecheck` / `${BUN_EXECUTABLE} run lint` in `cronymax_web_check` target
- [x] 3.6 Update `COMMENT` strings in both custom targets to reference bun

## 4. CI pipeline

- [x] 4.1 In `.github/workflows/release.yml` (`web-build` job): remove `pnpm/action-setup@v4` step
- [x] 4.2 Remove `actions/setup-node@v4` step (and its `cache: "pnpm"` option)
- [x] 4.3 Add `oven-sh/setup-bun@v2` step with `bun-version: "1.3.13"`
- [x] 4.4 Replace `pnpm install --frozen-lockfile` with `bun install --frozen-lockfile`
- [x] 4.5 Replace `pnpm --filter cronymax-web typecheck` with `bun --filter cronymax-web run typecheck`
- [x] 4.6 Replace `pnpm --filter cronymax-web lint` with `bun --filter cronymax-web run lint`
- [x] 4.7 Replace `pnpm --filter cronymax-web build` with `bun --filter cronymax-web run build`
- [x] 4.8 Update the job comment header to reflect bun

## 5. Documentation

- [x] 5.1 Update `README.md` prerequisites table: replace pnpm row with bun
- [x] 5.2 Update README build instructions: replace all `pnpm install --frozen-lockfile` and `pnpm dev/build/typecheck/lint/preview` references with bun equivalents
- [x] 5.3 Update README "Frontend development" section command examples

## 6. Verification

- [x] 6.1 Run `bun install` from repo root — confirm no errors, `bun.lock` generated, `@vscode/codicons` workspace link resolves
- [x] 6.2 Run `bun run build` from `web/` — confirm Vite produces `web/dist/` with all panel entries
- [x] 6.3 Run `bun run typecheck` from `web/` — confirm tsc passes
- [x] 6.4 Run `bun run lint` from `web/` — confirm ESLint passes
- [x] 6.5 Run `bun run test` from `web/` — confirm Vitest passes
- [ ] 6.6 Run `cmake --build build --target cronymax_web` — confirm CMake finds bun and the web build completes
- [ ] 6.7 Confirm the running app loads panels correctly from `file://` (SVG imports via `vite-plugin-svgr` resolved through bun's linker)
