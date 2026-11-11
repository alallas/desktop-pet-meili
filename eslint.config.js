// ESLint flat config（工程规范 §6：eslint + tsc --noEmit + vitest run）
import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/", "node_modules/", "src-tauri/", "src/ipc/types.ts"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.ts"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.es2024 },
    },
    rules: {
      // 工程规范 §2 TS：禁止 any（不得已 unknown + 收窄）
      "@typescript-eslint/no-explicit-any": "error",
      // 调试输出统一走 Rust debug_log，不进 console
      "no-console": "error",
    },
  }
);
