import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "../../walker/css";

const result = createResultItem({
  subname: "",
  description: "使用了 position: fixed",
  advice: "注意将其移动到 scroll-view 外",
  level: RuleLevel.Warn,
});

export default defineRule({ name: "position-fixed", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (
        isType(node, "Declaration") &&
        node.property === "position" &&
        isType(node.value, "Value") &&
        node.value.children.some((val) => isType(val, "Identifier") && val.name === "fixed")
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
