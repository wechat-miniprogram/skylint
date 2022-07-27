import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";
import { Identifier, List, Value } from "css-tree";

const result = createResultItem({
  subname: "",
  description: "flex 布局下未显式指定 flex-direction",
  advice: "目前 flex-direction 默认为 column，而在 web 下默认值为 row 故一般不会显式指定，因此这里需要显式指定为 row",
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
              path: ctx.env.path,
            },
          });
          ctx.addPatch({
            loc: {
              start: loc.start.offset,
              end: loc.start.offset,
              path: ctx.env.path,
            },
            patchedStr: "flex-direction: row; ",
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
