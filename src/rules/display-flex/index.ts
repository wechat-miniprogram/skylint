import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "../../walker/css";
import { Identifier, List, Value } from "css-tree";

const result = createResultItem({
  subname: "",
  description: "未显式指定 flex-direction",
  advice: "默认值为 column",
  fixable: true,
  level: RuleLevel.Warn,
});

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

          ctx.addPatch({
            loc: {
              start: node.loc!.start.offset,
              end: node.loc!.end.offset,
            },
            patchedStr: "\nflex-direction: row\n",
          });
          ctx.addASTPatch(() => {
            let children = new List<Identifier>();
            children.appendData({
              type: "Identifier",
              name: "row",
            });
            let value: Value = {
              type: "Value",
              children,
            };
            node.children.push({
              type: "Declaration",
              property: "flex-direction",
              important: false,
              value,
            });
          });
        }
      }
    },
  });
});
