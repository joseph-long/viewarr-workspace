import * as esbuild from 'esbuild';
import * as fs from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Read the WASM file and encode as base64
const wasmPath = path.join(__dirname, 'viewarr', 'pkg', 'viewarr_bg.wasm');
const wasmBytes = fs.readFileSync(wasmPath);
const wasmBase64 = wasmBytes.toString('base64');

// Plugin to replace the WASM URL loader with inline base64
const inlineWasmPlugin = {
  name: 'inline-wasm',
  setup(build) {
    // Intercept imports to viewarr.js and inject the WASM initialization
    build.onLoad({ filter: /viewarr\.js$/ }, async (args) => {
      let contents = fs.readFileSync(args.path, 'utf8');
      
      // Replace the URL-based WASM loading with inline base64 decoding
      // Find the __wbg_init function and modify it
      contents = contents.replace(
        /if \(module_or_path === undefined\) \{[\s\S]*?module_or_path = new URL\('viewarr_bg\.wasm', import\.meta\.url\);[\s\S]*?\}/,
        `if (module_or_path === undefined) {
    // Decode inline base64 WASM
    const wasmBase64 = "${wasmBase64}";
    const wasmBinary = Uint8Array.from(atob(wasmBase64), c => c.charCodeAt(0));
    module_or_path = wasmBinary.buffer;
  }`
      );
      
      return { contents, loader: 'js' };
    });
  },
};

await esbuild.build({
  entryPoints: ['js/widget.ts'],
  bundle: true,
  format: 'esm',
  minify: process.argv.includes('--dev') ? false : true,
  sourcemap: process.argv.includes('--dev') ? 'inline' : false,
  outdir: 'src/pyviewarr/static',
  plugins: [inlineWasmPlugin],
  logLevel: 'info',
});
