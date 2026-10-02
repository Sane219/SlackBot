import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Builds to `../dist`, which is committed and embedded in the Rust binary by build.rs.
// See docs/adr/0009-react-frontend.md.
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    // One bundle, one stylesheet. A local tool has no reason to lazy-load, and a
    // single hashed asset pair is two entries for build.rs to embed.
    rollupOptions: {
      output: {
        // Stable names, no content hash: the committed `dist/` diff is reviewable, and a
        // stale bundle is caught by CI rebuilding and comparing rather than by a
        // 404 in the browser.
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/app.[ext]",
      },
    },
    // Vite names the emitted stylesheet after the entry chunk, which would be `index`.
    // Pinning the order keeps the two asset names adjacent and obvious.
    cssCodeSplit: false,
  },
});