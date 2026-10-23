import { defineConfig } from "oxlint";

// HCCNA's server rules, without its anti-slop plugin (that comes when there is more
// TypeScript here).
export default defineConfig({
  categories: {
    correctness: "warn",
    suspicious: "warn",
    perf: "warn",
    style: "warn",
    pedantic: "error",
  },
  rules: {
    "no-unused-vars": [
      "error",
      {
        args: "after-used",
        argsIgnorePattern: "^_",
        vars: "all",
        varsIgnorePattern: "^_",
      },
    ],
    "require-await": "error",
    "sort-keys": "off",
    "no-ternary": "off",
    "one-var": "off",
    "id-length": "off",
    "sort-imports": "off",
    "capitalized-comments": "off",
    "max-statements": "off",
    "no-inline-comments": "off",
    "max-classes-per-file": "off",
    "no-null": "off",
    "new-cap": "off",
    "func-style": "off",
    "func-names": "off",
    "max-lines": "off",
    "no-magic-numbers": "off",
    "unicorn/filename-case": "off",
    // Effect's Schema.TaggedError is a class factory used in `extends` clauses.
    "unicorn/throw-new-error": "off",
    "max-lines-per-function": ["warn", { max: 600 }],
    "no-underscore-dangle": ["warn", { allow: ["_tag"] }],
  },
});
