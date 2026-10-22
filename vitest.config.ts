import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    environmentOptions: { jsdom: { pretendToBeVisual: true } },
    include: ["tests/*.test.tsx", "tests/*.test.ts"],
  },
});
