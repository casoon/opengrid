/**
 * Svelte 5 components for `@casoon/opengrid` (plan point 79, E32).
 *
 * Shipped as `.svelte` sources, the Svelte convention: the page's bundler
 * compiles them with the page's Svelte. The three components are thin: each
 * maps its props onto its element's attributes and hands the rest to
 * `Opengrid.svelte`, which renders the element and keeps one `connect`.
 */
export { default as OpengridGrid } from "./OpengridGrid.svelte";
export { default as OpengridTable } from "./OpengridTable.svelte";
export { default as OpengridPivot } from "./OpengridPivot.svelte";
