# ADR-0008: TypeScript 5.9

## Decision
Use TypeScript 5.9.x with `strict`, `noUncheckedIndexedAccess` and `verbatimModuleSyntax`, rather
than the native TypeScript 7 preview.

## Reason
Next.js 16, typescript-eslint and openapi-typescript are tested against 5.x. The native compiler
preview does not yet support every API those tools rely on. We will revisit once the ecosystem
supports it officially.
