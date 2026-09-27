# Sift

Sift is a privacy-first Windows file manager inspired by the three-part navigation of Files by Google: **Clean**, **Browse**, and **Share**. The frontend is a Tauri webview; every filesystem operation runs in Rust and is exposed through typed Tauri commands. It never asks for elevation and is confined to the current user's profile.

## What works

- **Browse:** Home, Desktop, Documents, Downloads, Pictures, Music, and Videos (when present); folder navigation; file metadata; cloud-placeholder badges; and debounced filename search across personal folders.
- **Clean:** an on-demand scan of personal folders, large-file discovery, and exact duplicate detection using BLAKE3. Duplicate review protects one copy in each matching set. Nothing is deleted without a selection and confirmation.
- **Recycle Bin:** selected items are passed to the platform Recycle Bin through the `trash` crate; Sift does not permanently delete user files.
- **Share:** create a random-token, local-network download link and QR code for one selected, locally available file. The link is limited to that file, expires after 20 minutes, and can be stopped early.
- **Desktop shell:** custom draggable titlebar, minimize/maximize/close controls, keyboard-accessible OS window snapping, light/dark/system appearance, responsive navigation, reduced-motion support, and virtualized long lists.

The browser preview intentionally shows no fabricated file records and cannot read local files. Real filesystem features are available in the Windows desktop build.

## Run

Requirements: Node.js 20+, Rust stable, and the [Tauri 2 Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows). From the repository root:

```sh
npm install
npm run dev       # browser preview at http://localhost:5173
npm run build     # strict TypeScript check and frontend production build
npm run tauri dev # Windows desktop app
npm run tauri build
```

The desktop window uses undecorated chrome; the window controls are implemented in Sift. Windows keyboard snapping (for example, **Win+Left/Right**) remains available.

## Filesystem safety

Rust validates every requested path against the current user's profile and Windows-known personal folders (including redirected folders), rejects parent traversal, checks each path component without following links, and uses the Windows extended-length path prefix for I/O. Windows reparse points are skipped; metadata flags for `OFFLINE`, `RECALL_ON_OPEN`, and `RECALL_ON_DATA_ACCESS` identify cloud-only items. Cloud items are shown as metadata-only and are never opened or hashed. Inaccessible entries are skipped and counted during enumeration. Content reads used for duplicate matching and sharing open a verified non-reparse file handle first.

The app does not traverse arbitrary drives or directories outside the current user's own profile. Nearby sharing listens only for the lifetime of the explicitly started session and exposes only the selected file behind a 192-bit random bearer token.

## Type boundary

Rust command DTOs derive `specta::Type`. `tauri-specta` exports the shared Rust DTOs to `src/lib/specta-bindings.ts` in debug desktop builds; the checked-in generated export keeps frontend type-checking available before the first desktop launch. `src/lib/bindings.ts` is the typed command-only bridge and consumes those generated DTOs.
