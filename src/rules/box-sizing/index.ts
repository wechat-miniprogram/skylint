import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const result = createResultItem({
  name: "box-sizing",
  description: "存在不支持的 box-sizing 值，skyline 只支持 border-box",
  advice: "改为 border-box",
  fixable: true,
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
        const loc = node.loc!;
        ctx.addResultWithPatch(
          {
            ...result,
            loc: {
              startLn: loc.start.line,
              endLn: loc.end.line,
              startCol: loc.start.column,
              endCol: loc.end.column,
              path: ctx.env.path,
            },
          },
          {
            loc: {
              start: loc.start.offset,
              end: loc.end.offset,
              path: ctx.env.path,
            },
            patchedStr: "box-sizing: border-box",
          }
        );
      }
    },
  });
});
