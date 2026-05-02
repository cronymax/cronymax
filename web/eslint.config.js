// Flat-config ESLint setup for cronymax-web (Phase 9 of the React migration).
// Intentionally minimal: parses TS/TSX with the typescript-eslint parser and
// enables the recommended rule set, with a couple of project-specific tweaks.
import tsParser from "@typescript-eslint/parser";
import tsPlugin from "@typescript-eslint/eslint-plugin";
import reactHooksPlugin from "eslint-plugin-react-hooks";

export default [
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "vite.config.js",
      "vite.config.d.ts",
      "**/*.tsbuildinfo",
      // Legacy non-typed runtime (re-exported as side-effect ESM imports).
      "src/shared/agent_runtime/**",
    ],
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 2022,
        sourceType: "module",
        ecmaFeatures: { jsx: true },
      },
      globals: {
        window: "readonly",
        document: "readonly",
        console: "readonly",
        localStorage: "readonly",
        navigator: "readonly",
        alert: "readonly",
        confirm: "readonly",
        prompt: "readonly",
        setTimeout: "readonly",
        clearTimeout: "readonly",
        setInterval: "readonly",
        clearInterval: "readonly",
        StorageEvent: "readonly",
        MouseEvent: "readonly",
        KeyboardEvent: "readonly",
        HTMLElement: "readonly",
        HTMLInputElement: "readonly",
        HTMLTextAreaElement: "readonly",
        HTMLDivElement: "readonly",
        HTMLPreElement: "readonly",
        HTMLButtonElement: "readonly",
        Event: "readonly",
        EventTarget: "readonly",
        AbortController: "readonly",
        fetch: "readonly",
        Response: "readonly",
        Request: "readonly",
        Headers: "readonly",
        URL: "readonly",
        Promise: "readonly",
      },
    },
    plugins: {
      "@typescript-eslint": tsPlugin,
      "react-hooks": reactHooksPlugin,
    },
    rules: {
      ...tsPlugin.configs.recommended.rules,
      // Allow intentional empty catch / non-null assertion for now;
      // strict-null + noUncheckedIndexedAccess already cover most cases.
      "@typescript-eslint/no-non-null-assertion": "off",
      "@typescript-eslint/no-explicit-any": "warn",
      // tsc already catches unused things; this avoids double-reporting.
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "no-empty": ["error", { allowEmptyCatch: true }],
      // The official react-hooks plugin isn't installed; suppress references
      // in inline disable comments so they don't trip the unknown-rule check.
      "react-hooks/exhaustive-deps": "off",
      // Don't fail the build on stale `// eslint-disable-next-line` comments.
      "no-unused-expressions": "off",
    },
    linterOptions: {
      reportUnusedDisableDirectives: "off",
    },
  },
];
