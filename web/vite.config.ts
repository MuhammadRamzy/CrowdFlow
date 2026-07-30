import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Cross-origin isolation is required for SharedArrayBuffer, which is required
// for WASM threads (docs/01-architecture.md §7). The engine runs
// single-threaded today, but serving these headers from the start means the
// constraint is discovered now — not after every third-party embed has been
// added and one of them turns out to break isolation.
const crossOriginIsolation = {
  name: 'cross-origin-isolation',
  configureServer(server: { middlewares: { use: (fn: unknown) => void } }) {
    server.middlewares.use((_req: unknown, res: { setHeader: (k: string, v: string) => void }, next: () => void) => {
      res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
      res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
      next();
    });
  },
};

export default defineConfig({
  plugins: [react(), crossOriginIsolation],
  server: {
    port: 5173,
    host: true,
    // fixtures/ is the shared test-venue directory used by both tracks and
    // lives above web/. Importing from there keeps one source of truth rather
    // than a copy that silently drifts.
    fs: { allow: ['..'] },
  },
  build: {
    target: 'es2022',
    // The wasm bundle is already optimised for size by wasm-pack; do not let
    // Vite inline it into JS, which would defeat streaming compilation.
    assetsInlineLimit: 0,
  },
  // wasm-pack output is generated, not authored — exclude it from dep scanning
  // so a rebuild does not require restarting the dev server.
  optimizeDeps: { exclude: ['./src/engine/cf_wasm.js'] },
});
