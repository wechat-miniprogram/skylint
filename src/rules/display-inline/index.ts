import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "../../walker/css";

const result = createResultItem({
  subname: "",
  description: "存在不支持的 display: inline",
  advice: "改为 text 文本嵌套",
  level: RuleLevel.Error,
});

export default defineRule({ name: "display-inline", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (
        isType(node, "Declaration") &&
        node.property === "display" &&
        isType(node.value, "Value") &&
        node.value.children.some((val) => isType(val, "Identifier") && val.name === "inline")
      ) {
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
