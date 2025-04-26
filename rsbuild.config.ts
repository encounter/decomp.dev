import { defineConfig } from '@rsbuild/core';
import { pluginSass } from '@rsbuild/plugin-sass';
import { pluginTypeCheck } from '@rsbuild/plugin-type-check';

export default defineConfig({
  source: {
    entry: {
      main: ['./js/main.ts', './css/main.scss'],
      history: ['./js/history.ts', './css/history.css'],
      manage: ['./js/manage.ts'],
      report: ['./js/treemap.ts'],
    },
  },
  // Write manifest.json, we pull JS and CSS asset paths from it
  output: {
    manifest: true,
  },
  // Disable HTML generation
  tools: {
    htmlPlugin: false,
  },
  plugins: [pluginSass(), pluginTypeCheck()],
  server: {
    port: 3001,
  },
  dev: {
    // Load assets directly from dev server
    assetPrefix: 'http://localhost:<port>/',
    // Connect HMR directly to dev server
    client: {
      host: 'localhost',
      port: '<port>',
    },
    // Always write dist/manifest.json
    writeToDisk: (file) => file === 'manifest.json',
  },
});
