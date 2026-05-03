# Cronymax Icons

Source of truth for every icon used by both the native (CEF Views) chrome
and the React panels. Sourced from [VS Code Codicons](https://github.com/microsoft/vscode-codicons),
MIT licensed.

## Pinned version

`@vscode/codicons` **0.0.45** (vendored 2026-05).

## Icon → IconId mapping

The native side defines the canonical `IconId` enum in
`app/browser/icon_registry.h`. The React side mirrors the same set as a
`type IconName` string union in `web/src/shared/icons.ts`.

| File                     | `IconId`       | React `IconName`     | Used for                               |
| ------------------------ | -------------- | -------------------- | -------------------------------------- |
| `arrow-left.svg`         | `kBack`        | `arrow-left`         | Web tab back button                    |
| `arrow-right.svg`        | `kForward`     | `arrow-right`        | Web tab forward button                 |
| `refresh.svg`            | `kRefresh`     | `refresh`            | Web tab refresh, terminal retry        |
| `debug-stop.svg`         | `kStop`        | `debug-stop`         | Web tab stop (refresh while loading)   |
| `add.svg`                | `kNewTab`      | `add`                | Web tab toolbar new-tab button         |
| `close.svg`              | `kClose`       | `close`              | Sidebar / settings / popover close     |
| `settings-gear.svg`      | `kSettings`    | `settings-gear`      | Title bar Settings button, agent rows  |
| `terminal.svg`           | `kTabTerminal` | `terminal`           | Terminal tab leading icon, sidebar row |
| `comment-discussion.svg` | `kTabChat`     | `comment-discussion` | Chat tab leading icon, sidebar row     |
| `type-hierarchy.svg`     | `kTabGraph`    | `type-hierarchy`     | Graph tab leading icon, sidebar row    |
| `globe.svg`              | `kTabWeb`      | `globe`              | Title bar Web button, web row fallback |
| `sparkle.svg`            | —              | `sparkle`            | Terminal "Explain" action              |
| `tools.svg`              | —              | `tools`              | Terminal "Fix" action                  |
| `save.svg`               | —              | `save`               | FlowEditor save                        |
| `trash.svg`              | —              | `trash`              | FlowEditor delete                      |
| `link-external.svg`      | —              | `link-external`      | Popover "Open as tab"                  |

`kTabAgent` reuses `settings-gear.svg`. `kRestart` reuses `refresh.svg`.

## Update procedure

1. `cd web && pnpm update @vscode/codicons` (or `pnpm add @vscode/codicons@<version>`).
2. Update the pinned version near the top of this file.
3. From the workspace root, recopy the listed SVGs:

   ```bash
   for f in arrow-left arrow-right refresh close add settings-gear terminal \
            comment-discussion type-hierarchy globe debug-stop sparkle tools \
            save trash link-external; do
     cp "web/node_modules/@vscode/codicons/src/icons/$f.svg" "assets/icons/$f.svg"
   done
   ```

4. Rebuild and visually confirm icons render.

## Bundling

`cmake/CronymaxApp.cmake` copies `assets/icons/*.svg` into
`cronymax.app/Contents/Resources/icons/` at build time. `IconRegistry::Init()`
reads them from there at startup and rasterises each into a `CefImage`.
