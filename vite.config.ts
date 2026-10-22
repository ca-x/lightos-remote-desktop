import { defineConfig } from "vite";

export default defineConfig({
  root: "web",
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    target: "es2022",
    rolldownOptions: {
      onwarn(warning, warn) {
        if (warning.code === "MODULE_LEVEL_DIRECTIVE" && warning.message.includes('"use client"')) return;
        warn(warning);
      },
      output: {
        entryFileNames: "app.js",
        codeSplitting: false,
        assetFileNames: "[name][extname]",
      },
    },
  },
});
