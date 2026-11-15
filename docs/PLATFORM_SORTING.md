# Platform layout sorting

Platform entries and group children use the same `SortableLayoutList` and
`SortableLayoutItem` components, with an isolated dnd-kit context for each list.
The existing order persistence methods run once on a successful drop. Pointer
movement only updates visual transforms; cancellation leaves the order intact.

The handle activates after five pixels of movement. DragOverlay shows an inert
DOM snapshot of the source, including expanded groups and custom icons, without
mounting duplicate React controls. Neighboring items animate into position and
the original item becomes a placeholder. Pointer drops must hit a row in the
same list. A release outside the rows cancels the drag. Keyboard sorting uses
the nearest eligible row.
The dialog's native scroll viewport supports edge auto-scroll.

Keyboard users can focus a handle, press Space, move with arrow keys and confirm
with Space or Enter. Escape cancels the drag. Its consumed key event does not
also close the dialog. Drop and displacement animations honor reduced motion.

Run `npm run test:platform-sort` for the isolated browser fixture. It uses the
real platform layout dialog with mock native preferences;
no user accounts or native application data are accessed. The checks cover
preview, displacement, delayed persistence, cancellation, keyboard, auto-scroll,
group child isolation and expanded group movement. `PLATFORM_SORT_REPORT` optionally
selects a JSON output file. `PLATFORM_SORT_BROWSER` selects a Chromium executable;
otherwise it looks for an installed Edge or Chrome/Chromium browser.

dnd-kit is MIT licensed; the installer includes `licenses/dnd-kit.txt`.
