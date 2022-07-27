import { defineRule, RuleType, RuleResultItem } from "../interface";
import { isType } from "../../walker/css";

const result: RuleResultItem = {
  subname: "",
  description: "存在不支持的 display: inline-block",
  advice: "改为 display: flex + flex-direction: row",
};

export default defineRule({ name: "display-inline-block", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (
        isType(node, "Declaration") &&
        node.property === "display" &&
        isType(node.value, "Value") &&
        node.value.children.some((val) => isType(val, "Identifier") && val.name === "inline-block")
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
