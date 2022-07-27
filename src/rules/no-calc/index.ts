import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const result = createResultItem({
  subname: "",
  description: "不支持 calc 表达式",
  advice: `需要改为静态值，可考虑使用兼容写法，即 height: 100px; height: calc(50px+3rem); 使得 skyline 下使用静态值，webview 下使用 calc 函数`,
  level: RuleLevel.Error,
  withCodeFrame: true,
});

export default defineRule({ name: "no-calc", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "Function") && node.name === "calc") {
        const loc = node.loc!;
        ctx.addResult({
          ...result,
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path,
          },
        });
      }
    },
  });
});
