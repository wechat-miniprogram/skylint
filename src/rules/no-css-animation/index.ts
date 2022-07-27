import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const result = createResultItem({
  subname: "",
  description: "不支持 css animation",
  advice: "可通过 skyline 的新特性，worklet 动画实现",
  level: RuleLevel.Warn,
});

export default defineRule({ name: "no-css-animation", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "Declaration") && node.property.startsWith("animation")) {
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
