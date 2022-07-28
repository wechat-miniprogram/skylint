import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const result = createResultItem({
  name: "text-overflow-ellipse",
  description: "text-overflow: ellipse 只在 text 组件下生效",
  advice: "文本省略需要通过 text 组件 + text-overflow: ellipse + overflow: hidden 三者实现",
  level: RuleLevel.Warn,
});

export default defineRule({ name: "text-overflow-ellipse", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node, walkCtx) => {
      if (isType(node, "Declaration") && node.property === "text-overflow") {
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
