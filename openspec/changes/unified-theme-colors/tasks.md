## 1. Theme Token Foundation

- [x] 1.1 Define the semantic Light and Dark token sets in `web/src/styles/theme.css` for brand, backgrounds, fills, text, lines, and function colors.
- [x] 1.2 Add a temporary compatibility mapping from existing `cronymax-*` tokens to the new semantic token names.
- [x] 1.3 Update shared theme typing and bridge payload shapes so resolved shell-relevant token values can be mirrored to native code.

## 2. Shell Chrome Surfaces

- [x] 2.1 Update native theme plumbing in `app/browser/main_window.*` and related bridge paths so title bar, sidebar, and window background all resolve from `bg_body`.
- [x] 2.2 Update the content frame and floating surfaces to use `bg_base`, `bg_float`, `bg_mask`, `border`, `text_title`, and `text_caption` instead of local color literals.
- [x] 2.3 Verify the shell chrome remains stable across theme changes and is not recolored by page-driven adaptation signals.

## 3. Adaptive Content Theme

- [x] 3.1 Formalize page-color sampling precedence so `theme-color` wins, body background is fallback, and missing signals revert to token-driven neutral content presentation.
- [x] 3.2 Constrain sampled page colors so low-contrast or extreme values are clamped or rejected before application.
- [x] 3.3 Limit adaptation to content-local presentation paths so outer shell chrome and frame boundaries stay token-driven.

## 4. Panel Migration

- [x] 4.1 Migrate sidebar, settings, and other shell-adjacent panels from legacy token names to the semantic token taxonomy.
- [x] 4.2 Migrate browser/tab-local styling paths that currently depend on old surface names to the new background, fill, and text roles.
- [x] 4.3 Remove temporary compatibility aliases once all affected panels consume the semantic token system directly.

## 5. Verification and Documentation

- [x] 5.1 Validate Light and Dark palettes against the teal-mint brand direction and confirm semantic status colors remain visually distinct.
- [ ] 5.2 Smoke-test theme switching and page adaptation behavior to confirm unified shell backgrounds, layered surfaces, and bounded content harmonization.
- [x] 5.3 Update architecture or theme documentation to reflect the semantic taxonomy, shell surface roles, and adaptive-content rules.
