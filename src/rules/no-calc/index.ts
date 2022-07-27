import { defineRule, RuleType, RuleResultItem } from "../interface";
import { isType } from "../../walker/css";

const result: RuleResultItem = {
  subname: "",
  description: "存在不支持的 calc 表达式",
  advice: "改为静态值",
};

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
          },
        });
      }
    },
  });
});
