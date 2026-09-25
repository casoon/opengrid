import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite";

// A development build on purpose (plan point 78): Vue's warnings exist only
// there, and the tests fail on any of them.
export default defineConfig({
  base: "./",
  plugins: [vue()],
  define: {
    "process.env.NODE_ENV": JSON.stringify("development"),
    __VUE_PROD_DEVTOOLS__: "false",
    __VUE_OPTIONS_API__: "true",
    __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: "false",
  },
  build: { emptyOutDir: true, minify: false },
});
