import path from "path";
import { defineBuildConfig } from "unbuild";

export default defineBuildConfig({
  entries: ["./src/index", "./src/cli"],
  rootDir: "./",
  rollup: {
    alias: {
      entries: {
        src: path.resolve("./src"),
      },
    },
  },
  outDir: "dist",
  declaration: true,
});
