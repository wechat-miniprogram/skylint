import { defineRule, RuleType, RuleResultItem } from "../interface";
import { isType } from "../../walker/css";

const result: RuleResultItem = {
  description: "存在不支持的 box-sizing: content-box",
  advice: "改为 border-box",
};

export default defineRule({ name: "box-sizing", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (
        isType(node, "Declaration") &&
        node.property === "box-sizing" &&
        isType(node.value, "Value") &&
        node.value.children.some((val) => isType(val, "Identifier") && val.name === "content-box")
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
