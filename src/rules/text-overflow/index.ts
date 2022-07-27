import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "../../walker/css";

const result = createResultItem({
  subname: "",
  description: "仅 text 组件支持 text-overflow 属性",
  advice: "检查该规则是否应用于非 text 组件之上",
  level: RuleLevel.Warn,
});

export default defineRule({ name: "text-overflow", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    before: () => {
      console.log('wtf')
    },
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
          },
        });
      }
    },
  });
});
