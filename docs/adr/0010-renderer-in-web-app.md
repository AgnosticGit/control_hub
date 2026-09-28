# ADR-0010: Pixi renderer lives in apps/web

## Decision
The PixiJS renderer (`apps/web/src/renderer/pixi-map.ts`) stays in the web app instead of a
separate `packages/renderer`. It depends only on `@controlhub/web-engine` constants (strides and
flags) and on pixi.js, and it is loaded with a dynamic `import()` from the map component.

## Reason
There is one consumer. A separate package would add build wiring without any reuse. It can be
extracted once a second consumer exists, for example an embeddable widget.
