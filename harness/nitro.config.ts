import { defineNitroConfig } from "nitro/config";

// `workflow/nitro` 是编译 `"use workflow"` / `"use step"` 的地方：
// 没有它，`start()` 会报 "invalid workflow function"。
export default defineNitroConfig({
  modules: ["workflow/nitro"],
  routes: {
    "/**": { handler: "./src/server.ts", format: "node" },
  },
});
