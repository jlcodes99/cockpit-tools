# Application scrollbars

The application uses OverlayScrollbars 2 for its scroll owners: the sidebar,
page content, dialogs, tables and menus. Textareas intentionally keep their
native scrollbars. The installer includes the dependency's MIT license.

`installAppScrollbars` runs once per frontend window. It discovers native
`overflow: auto/scroll` elements and observes mounted content, size changes,
input and lazy page styles. It keeps the existing element as the viewport:
React children, flex/grid layout, refs, `scrollTop`, native scroll events and
modal overflow locks retain their behavior. A cascade layer preserves the
application's existing positioning and overflow styles.

Scrollbar controls live in a separate layer outside the React tree. Their
geometry follows the viewport and clipping ancestors; hit testing hides
controls occluded by dialogs. Dropdown outside-click and focus handlers use
`containsWithScrollbar` so dragging an external handle belongs to its owner.

The `os-theme-cockpit` theme uses the application's light/dark color variables.
It supports both axes, dragging and track clicks, with automatic hiding.
Platform pins remain unlimited and retain their original size when scrolling.

Run `npm run test:scrollbars` for a headless browser integration check. It uses
Microsoft Edge on Windows, or `SCROLLBARS_BROWSER` as the Chromium executable.
It verifies native scrolling/events, layout, React reconciliation, textareas,
handle dragging, a real portaled dropdown, modal occlusion and disposal.
`SCROLLBARS_REPORT` optionally writes the JSON results to a chosen file.
No application accounts or network services are used by this fixture.
