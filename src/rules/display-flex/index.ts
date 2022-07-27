import { defineRule, RuleType, RuleResultItem } from "../interface";
import { isType } from "../../walker/css";

const result: RuleResultItem = {
  description: "未显式指定 flex-direction",
  advice: "默认值为 column",
};

export default defineRule({ name: "display-flex", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "Block")) {
        let loc: typeof node.loc;
        let hasFlexDirection = false;
        node.children.forEach((child) => {
          if (!isType(child, "Declaration")) return;
          if (
            child.property === "display" &&
            isType(child.value, "Value") &&
            child.value.children.some((val) => isType(val, "Identifier") && val.name === "flex")
          ) {
            loc = child.loc;
          }
          if (child.property === "flex-direction") hasFlexDirection = true;
        });
        if (loc && !hasFlexDirection) {
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
      }
    },
  });
});
