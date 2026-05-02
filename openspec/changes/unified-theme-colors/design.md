## Context

The current app theme is split across native constants, a small renderer token set, and a separate per-tab chrome tint path. That split is manageable for a single dark palette, but it breaks down once the product needs a deliberate teal-mint brand system, a full Light/Dark taxonomy, and a stable rule for when webpage colors can influence the visible content area.

This change cuts across native shell layout, renderer styling, bridge payloads, and per-tab color sampling. The design therefore needs to separate three concerns that are currently conflated: the canonical token system, the shell chrome surfaces that must stay unified, and the content-surface adaptation rules for web pages.

## Goals / Non-Goals

**Goals:**

- Establish one canonical token taxonomy for Light and Dark themes covering brand, backgrounds, fills, text, lines, and semantic function colors.
- Rebase the brand palette around teal-mint while keeping status colors distinct from the brand axis.
- Make title bar, sidebar, and native window background resolve from the same shell token so the chrome reads as one continuous surface.
- Define a layered surface model where `bg_body`, `bg_base`, `bg_float`, and `bg_mask` have clear and consistent roles across native and web UI.
- Preserve a stable shell chrome even when a webpage provides `theme-color` or a strong body background.
- Make adaptive content color deterministic enough to specify and test.

**Non-Goals:**

- No redesign of every panel's component layout or information architecture.
- No per-site or per-tab saved theme customization beyond the existing runtime sampling behavior.
- No full-window recoloring driven by arbitrary webpage colors.
- No new dependency on an external design-token build system.
- No attempt to solve unrelated sidebar navigation, settings, or title-bar layout issues beyond their color contract.

## Decisions

### Decision 1 - CSS custom properties remain the source of truth for semantic tokens

The token system will live in `web/src/styles/theme.css` as semantic CSS custom properties, not as a native-only color table. The taxonomy will be expanded to the user-facing names discussed during exploration: `primary`, `secondary`, `bg_body`, `bg_base`, `bg_float`, `bg_mask`, `fill_*`, `text_*`, `border`, `divider`, `info`, `success`, `warning`, and `error`.

The renderer remains the place where token naming and composition live. Native code only consumes a small mirrored subset required to paint shell surfaces.

Alternatives considered:

- Keep the current `cronymax-*` names and only swap hex values. Rejected because that preserves the current ambiguity between shell background, card background, and floating surfaces.
- Move all token resolution into C++. Rejected because token composition is already web-first and would become harder to evolve.

### Decision 2 - Teal is the primary action signal; mint is supportive, not dominant

The Light and Dark palettes will use teal as `primary` and mint as `secondary`. Interaction tokens such as `fill_active`, `fill_focus`, `fill_hover`, `fill_pressed`, and `fill_selected` derive from teal-dominant values. Mint is reserved for softer companion roles such as tags, supportive highlights, and secondary emphasis.

This avoids a system where active state, success state, and decorative accent all collapse into similar greens.

Alternatives considered:

- Split action emphasis evenly across teal and mint. Rejected because active state becomes visually ambiguous.
- Keep purple as the main accent and add teal only as secondary. Rejected because it conflicts with the new brand direction.

### Decision 3 - Shell chrome is token-driven and must not follow webpage colors

The shell chrome contract is strict: title bar, sidebar, and native window background are painted from `bg_body`; the content frame uses `bg_base` and `border`; popovers and menus use `bg_float`; overlays use `bg_mask`. Native code mirrors only the tokens required to enforce that contract.

This means external page colors never repaint the outer shell. They can only influence the content region inside the content frame.

Alternatives considered:

- Allow page color to recolor the entire content host including the outer frame. Rejected because it conflicts with the requirement for unified chrome and makes the app identity unstable.
- Disable page-color adaptation entirely. Rejected because content harmonization remains valuable for browser tabs.

### Decision 4 - Adaptive content theme affects the inner content presentation, not the chrome frame

The existing `tab.set_chrome_theme` sampling path remains the starting point for page-aware color, but its responsibility is narrowed and formalized. The system samples `meta[name="theme-color"]` first, then non-transparent `body` background, and produces a harmonized content accent for the active tab. That accent may tint inner content presentation, toolbar treatment, or a derived content-surface layer, but it must remain clamped so text contrast and shell separation are preserved.

The content frame itself remains grounded in the app's neutral tokens. This gives the browser tab room to feel integrated with the page without sacrificing a stable app shell.

Alternatives considered:

- Treat page color as a direct replacement for `bg_base`. Rejected because bright or low-contrast pages would degrade readability.
- Use only `theme-color` and ignore body background. Rejected because many pages still omit the meta tag.

### Decision 5 - Migration uses semantic aliases before full cleanup

Although this change is marked as breaking at the token-contract level, implementation should migrate through a short alias period. Existing `cronymax-*` tokens can temporarily map onto the new semantic tokens so panel styles can be updated incrementally without visual regression. The compatibility layer is transitional and should be removed once panels stop depending on the old names.

Alternatives considered:

- One-shot rename of every token consumer. Rejected because it increases rollout risk across native/web boundaries.
- Permanent dual naming. Rejected because it would recreate the ambiguity this change is trying to remove.

## Risks / Trade-offs

- [Semantic token expansion creates a large migration surface across panels.] -> Use a temporary alias layer and migrate high-traffic surfaces first.
- [Page-driven content harmonization may reduce contrast on unusual sites.] -> Clamp adapted colors to a bounded tonal range and fall back to neutral content tokens when contrast is insufficient.
- [Native and renderer surfaces may drift if token mirroring is incomplete.] -> Limit the native mirror to a small documented subset and keep names aligned with the semantic taxonomy.
- [Teal brand colors could be confused with success states.] -> Keep success on a separate green track and reserve mint for supportive emphasis rather than primary actions.
- [The new taxonomy may be overkill for panels that only need a few colors.] -> Accept the larger taxonomy because the consistency benefit outweighs the extra names.

## Migration Plan

1. Add the new semantic token set for Light and Dark themes in the renderer and map old `cronymax-*` variables to semantic aliases.
2. Update native theme plumbing to consume the shell-specific subset (`bg_body`, `bg_base`, `border`, `text_title`, `text_caption`, `bg_float`, `bg_mask`) from resolved theme payloads.
3. Migrate title bar, sidebar, content frame, and floating surfaces to semantic tokens.
4. Tighten the adaptive page-color path so it only influences content-local presentation and cannot repaint shell chrome.
5. Remove temporary aliases after panel styles no longer reference the old token names.

Rollback is a single change revert because no persistent data migration is required.

## Open Questions

- Should adapted page color tint only the browser toolbar, or also a content-local background layer inside the frame?
- Do we want explicit numeric contrast thresholds for accepting or rejecting a sampled page color in the first version?
- Should the final semantic token names use snake_case exactly as proposed here, or be exposed in CSS with a prefixed kebab-case form that preserves the same conceptual grouping?
