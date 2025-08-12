import { defineConfig } from '@rsbuild/core';
import { pluginReact } from '@rsbuild/plugin-react';
import { pluginSass } from '@rsbuild/plugin-sass';
import { pluginTypeCheck } from '@rsbuild/plugin-type-check';

export default defineConfig({
  source: {
    entry: {
      entry: ['./js/entry.ts'],
      main: ['./js/main.ts', './css/main.scss'],
      history: ['./js/history.ts', './css/history.css'],
      manage: ['./js/manage.ts'],
      report: ['./js/treemap.ts'],
      api: ['./js/api.tsx'],
      projects: ['./js/projects.ts'],
    },
  },
  output: {
    // Write manifest.json, we pull JS and CSS asset paths from it
    manifest: true,
    // Copy public files to dist
    copy: [{ from: 'public' }],
  },
  tools: {
    // Disable HTML generation
    htmlPlugin: false,
    rspack: {
      // Use shared runtime for all entry points
      optimization: {
        runtimeChunk: 'single',
      },
      // Disable bundler info to save a few bytes
      experiments: {
        rspackFuture: {
          bundlerInfo: {
            force: false,
          },
        },
      },
    },
  },
  plugins: [pluginSass(), pluginTypeCheck(), pluginReact()],
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
      // Error overlay doesn't work with CSP
      overlay: false,
    },
    // Always write dist/manifest.json and public files
    writeToDisk: (file) => {
      return (
        file.endsWith('manifest.json') ||
        (!file.includes('static/') && !file.includes('hot-update'))
      );
    },
  },
});
