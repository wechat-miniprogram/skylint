import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

export default defineRule({ name: "display-inline", type: RuleType.WXSS }, (ctx) => {
  ctx.lifetimes({
    onVisit: (node) => {
      if (isType(node, "Declaration") && node.property === "display" && isType(node.value, "Value")) {
        const val = node.value.children
          .toArray()
          .find((val) => isType(val, "Identifier") && ["inline", "inline-block"].includes(val.name));
        if (!val || !isType(val, "Identifier")) return;
        const loc = node.loc!;
        ctx.addResult({
          name: `display-${val.name}`,
          description: `不支持 display: ${val.name}`,
          advice: "若是布局需要，可改为 flex 布局实现；若是实现内联文本，可使用 text 组件",
          level: RuleLevel.Info,
          loc: {
            startLn: loc.start.line,
            endLn: loc.end.line,
            startCol: loc.start.column,
            endCol: loc.end.column,
            path: ctx.env.path,
          },
        });
      }
    },
  });
});
