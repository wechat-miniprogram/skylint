import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "../../walker/css";

const result = createResultItem({
  subname: "",
  description: "存在不支持的 box-sizing: content-box",
  advice: "改为 border-box",
  level: RuleLevel.Warn,
});

export default defineRule({ name: "box-sizing", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (
        isType(node, "Declaration") &&
        node.property === "box-sizing" &&
        isType(node.value, "Value") &&
        node.value.children.some((val) => isType(val, "Identifier") && val.name === "content-box")
      ) {
        const identifiers = node.value.children;
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
        ctx.addASTPatch(() => {
          identifiers.clear();
          identifiers.appendData({
            type: "Identifier",
            name: "border-box",
          });
        });
        ctx.addPatch({
          loc: {
            start: loc.start.offset,
            end: loc.end.offset,
          },
          patchedStr: "box-sizing: border-box",
        });
      }
    },
  });
});
