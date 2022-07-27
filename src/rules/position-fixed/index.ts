import { defineRule, RuleType, createResultItem, RuleLevel } from "../interface";
import { isType } from "src/walker/css";

const result = createResultItem({
  subname: "",
  description: "不支持 position: fixed",
  advice: "skyline 由于不支持全局滚动，故在页面根节点下使用 absolute 即可达到 fixed 的效果，若封装原因无法移至页面根节点，可使用 root-portal 组件包裹",
  level: RuleLevel.Error,
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
